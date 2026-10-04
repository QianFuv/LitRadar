package index

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"slices"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type batchTestRequest struct {
	Catalogs []struct {
		Path, Filename, Name, Digest, Provider string
		Entries                                []domain.JournalCatalogEntry
	}
	Selection   string
	Mode        domain.IndexSyncMode
	Size        uint64
	Notify, Dry bool
}

func (value batchTestRequest) request() (BatchRequest, error) {
	catalogs := []CatalogInput{}
	for _, item := range value.Catalogs {
		catalogs = append(catalogs, CatalogInput{item.Path, item.Filename, item.Name, item.Digest, item.Provider, item.Entries})
	}
	return NewBatchRequest(catalogs, value.Selection, value.Mode, value.Size, value.Notify, value.Dry)
}

type batchTestOperation struct {
	Op, Sql, Phase, Attempt, Status string
	Owner                           *string
	Now                             *int64
	Ordinal                         uint64
	Resume                          *bool
	Ack                             bool
	Exit                            *int32
	Request                         batchTestRequest
	Value                           struct {
		Run, Path, Generated string
		Digest               *string
		Journals, Attempts   uint64
		Written              int64
		Payload              []byte
		Through              *int64
	}
}

func (value batchTestOperation) outcome() BatchCatalogOutcome {
	var path *string
	if value.Value.Path != "" {
		path = &value.Value.Path
	}
	return BatchCatalogOutcome{value.Value.Run, value.Value.Journals, value.Value.Written, value.Value.Attempts, path}
}
func TestOriginalRustBatchWorkflows(t *testing.T) {
	body, err := os.ReadFile("../../../tests/migration/index/batch-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Name, Setup string
				Operations  []batchTestOperation
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
			database, err := sqlite.OpenMigration(filepath.Join(t.TempDir(), "batch.sqlite"))
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
			ids := []string{}
			if err := InitBatch(ctx, connection); err != nil {
				actual = map[string]any{"error": err.Error()}
			} else {
				current := "missing"
				operations := []any{}
				for _, operation := range item.Input.Operations {
					owner := "owner-1"
					if operation.Owner != nil {
						owner = *operation.Owner
					}
					now := int64(100)
					if operation.Now != nil {
						now = *operation.Now
					}
					var value any = map[string]any{"ok": true}
					var err error
					switch operation.Op {
					case "admit", "replace":
						requested, requestErr := operation.Request.request()
						err = requestErr
						if err == nil {
							if operation.Op == "admit" {
								var admission BatchAdmission
								admission, err = AdmitBatch(ctx, connection, requested, operation.Resume == nil || *operation.Resume, owner, now)
								if err == nil {
									current = admission.Batch.BatchId
									value = map[string]any{"abandoning": admission.IsAbandoning, "batch": batchTestView(admission.Batch, &ids)}
								}
							} else {
								var batch IndexBatch
								batch, err = ReplaceAbandoningBatch(ctx, connection, current, requested, owner, now)
								if err == nil {
									current = batch.BatchId
									value = batchTestView(batch, &ids)
								}
							}
						}
					case "heartbeat":
						err = HeartbeatBatchLease(ctx, connection, current, owner, now)
					case "release":
						err = ReleaseBatchLease(ctx, connection, current, owner)
					case "phase":
						err = TransitionCatalogPhase(ctx, connection, current, owner, operation.Ordinal, CatalogPhase(operation.Phase), now)
					case "outcome":
						err = StoreCatalogOutcome(ctx, connection, current, owner, operation.Ordinal, operation.outcome(), now)
					case "intent":
						var intent ManifestIntent
						intent, err = NewManifestIntent(operation.Value.Payload, operation.Value.Through, operation.Value.Path, operation.Value.Run, operation.Value.Generated)
						if err == nil {
							if operation.Value.Digest != nil {
								intent.Sha256 = *operation.Value.Digest
							}
							err = StoreManifestIntent(ctx, connection, current, owner, operation.Ordinal, intent, now)
						}
					case "notify_prepare":
						var result NotifyAttemptPreparation
						result, err = PrepareNotifyAttempt(ctx, connection, current, owner, operation.Ordinal, operation.Attempt, operation.Ack, now)
						value = map[string]any{"decision": result.Decision, "state": handoffTestView(result.State)}
					case "notify_record":
						var result NotifyHandoffState
						result, err = RecordNotifyAttemptResult(ctx, connection, current, owner, operation.Ordinal, operation.Attempt, NotifyStatus(operation.Status), operation.Exit, now)
						value = handoffTestView(result)
					case "complete_catalog":
						err = CompleteCatalog(ctx, connection, current, owner, operation.Ordinal, operation.outcome(), now)
					case "complete_batch":
						err = CompleteBatch(ctx, connection, current, owner, now)
					case "sql":
						_, err = connection.ExecContext(ctx, operation.Sql)
					case "read":
						var catalogs []IndexBatchCatalog
						catalogs, err = ReadBatchCatalogs(ctx, connection, current)
						value = catalogsTestView(catalogs)
					default:
						t.Fatal(operation.Op)
					}
					if err != nil {
						value = map[string]any{"error": err.Error()}
					}
					operations = append(operations, value)
				}
				var version int64
				if err := connection.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
					t.Fatal(err)
				}
				actual = map[string]any{"operations": operations, "tables": selectedSnapshot(t, connection, []string{"index_batches", "index_batch_catalogs", "index_batch_lease"}), "version": version}
			}
			encoded, err := json.Marshal(actual)
			if err != nil {
				t.Fatal(err)
			}
			var got, want any
			decoder := json.NewDecoder(bytes.NewReader(encoded))
			decoder.UseNumber()
			if err := decoder.Decode(&got); err != nil {
				t.Fatal(err)
			}
			decoder = json.NewDecoder(bytes.NewReader(item.Expected))
			decoder.UseNumber()
			if err := decoder.Decode(&want); err != nil {
				t.Fatal(err)
			}
			got = normalizeBatchTestIds(got, ids)
			if !reflect.DeepEqual(got, want) {
				normalized, _ := json.Marshal(got)
				t.Fatalf("actual=%s\nexpected=%s", normalized, item.Expected)
			}
		})
	}
}
func batchTestView(value IndexBatch, ids *[]string) any {
	if !slices.Contains(*ids, value.BatchId) {
		*ids = append(*ids, value.BatchId)
	}
	return map[string]any{"id": value.BatchId, "owner": value.OwnerId, "started": value.StartedAt, "resumed": value.DidResume, "catalogs": catalogsTestView(value.Catalogs)}
}
func handoffTestView(value NotifyHandoffState) any {
	return map[string]any{"attempt": value.AttemptId, "status": value.Status, "exit": value.ExitCode, "ack_attempt": value.UnknownAcknowledgedAttemptId, "ack_time": value.UnknownAcknowledgedAt}
}
func catalogsTestView(values []IndexBatchCatalog) any {
	result := []any{}
	for _, value := range values {
		var outcome, intent, handoff any
		if value.Outcome != nil {
			stored := value.Outcome
			outcome = map[string]any{"run": stored.RunId, "journals": stored.JournalCount, "written": stored.WrittenArticleCount, "attempts": stored.SourceAttemptCount, "path": stored.ManifestPath}
		}
		if value.ManifestIntent != nil {
			stored := value.ManifestIntent
			payload := []int{}
			for _, value := range stored.Payload {
				payload = append(payload, int(value))
			}
			intent = map[string]any{"payload": payload, "digest": stored.Sha256, "through": stored.ThroughEventId, "path": stored.Path, "run": stored.RunId, "generated": stored.GeneratedAt}
		}
		if value.NotifyHandoff != nil {
			handoff = handoffTestView(*value.NotifyHandoff)
		}
		result = append(result, map[string]any{"ordinal": value.Ordinal, "filename": value.Filename, "name": value.CatalogName, "provider": value.ProviderName, "journals": value.JournalCount, "phase": value.Phase, "outcome": outcome, "intent": intent, "handoff": handoff})
	}
	return result
}
func normalizeBatchTestIds(value any, ids []string) any {
	switch typed := value.(type) {
	case string:
		if ordinal := slices.Index(ids, typed); ordinal >= 0 {
			return fmt.Sprintf("batch-%d", ordinal+1)
		}
	case []any:
		for ordinal, item := range typed {
			typed[ordinal] = normalizeBatchTestIds(item, ids)
		}
	case map[string]any:
		for key, item := range typed {
			typed[key] = normalizeBatchTestIds(item, ids)
		}
	}
	return value
}
