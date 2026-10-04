package delivery

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

type durableStep map[string]json.RawMessage

func (step durableStep) text(name, fallback string) string {
	var value *string
	json.Unmarshal(step[name], &value)
	if value != nil {
		return *value
	}
	return fallback
}
func (step durableStep) integer(name string, fallback int64) int64 {
	var value *int64
	json.Unmarshal(step[name], &value)
	if value != nil {
		return *value
	}
	return fallback
}
func (step durableStep) real(name string, fallback float64) float64 {
	var value *float64
	json.Unmarshal(step[name], &value)
	if value != nil {
		return *value
	}
	return fallback
}
func (step durableStep) optionalText(name string) *string {
	var value *string
	json.Unmarshal(step[name], &value)
	return value
}
func (step durableStep) optionalInteger(name string) *int64 {
	var value *int64
	json.Unmarshal(step[name], &value)
	return value
}
func (step durableStep) optionalReal(name string) *float64 {
	var value *float64
	json.Unmarshal(step[name], &value)
	return value
}

func performDurableStep(ctx context.Context, repository *Repository, step durableStep) (any, error) {
	run, revision, now := step.integer("run", 1), step.integer("revision", 0), step.real("now", 100.25)
	owner, database, seconds := step.text("owner", "owner"), step.text("db", "fixture.sqlite"), step.real("seconds", 10)
	item, workflow := step.integer("item", 1), Workflow(step.text("workflow", "notify"))
	checkpoint := CheckpointUpdate{Status: CheckpointStatus(step.text("checkpoint_status", "completed")), SnapshotJson: step.text("snapshot", "{}"), LastCompletedRunAt: step.optionalText("completed_at"), UpdatedAt: now}
	var reservations []DedupeResolution
	var rawReservations []durableStep
	json.Unmarshal(step["reservations"], &rawReservations)
	for _, raw := range rawReservations {
		reservations = append(reservations, DedupeResolution{raw.integer("id", 1), raw.integer("revision", 0)})
	}
	result, errorCode, message := step.optionalText("result"), step.optionalText("error_code"), step.optionalText("message")
	var err error
	switch step.text("op", "") {
	case "admit", "admit_manual":
		db := step.optionalText("db_name")
		if _, exists := step["db_name"]; !exists {
			value := "fixture.sqlite"
			db = &value
		}
		create := RunCreate{ExternalId: step.text("external_id", "run"), Workflow: workflow, ScopeKey: step.text("scope_key", "fixture.sqlite"), DbName: db, TriggerKind: TriggerKind(step.text("trigger", "scheduled")), Mode: RunMode(step.text("mode", "execute")), UserId: step.optionalInteger("user_id"), DeadlineAt: step.optionalReal("deadline"), CreatedAt: now}
		var outcome RunOutcome
		if step.text("op", "") == "admit_manual" {
			outcome, err = repository.AdmitManualRun(ctx, create)
		} else {
			outcome, err = repository.AdmitRun(ctx, create)
		}
		if err != nil {
			return nil, err
		}
		return map[string]any{"kind": outcome.Kind, "id": outcome.Run.Id}, nil
	case "claim":
		outcome, err := repository.ClaimRun(ctx, run, owner, revision, now, seconds)
		if err != nil {
			return nil, err
		}
		return map[string]any{"kind": outcome.Kind, "id": outcome.Run.Id}, nil
	case "start":
		_, err = repository.StartRun(ctx, run, owner, revision, now)
	case "renew":
		_, err = repository.RenewRun(ctx, run, owner, revision, now, seconds)
	case "cancel":
		_, err = repository.CancelRun(ctx, run, revision, now)
	case "finalize":
		_, err = repository.FinalizeRun(ctx, run, owner, revision, RunStatus(step.text("status", "completed")), result, errorCode, now)
	case "finalize_queued":
		_, err = repository.FinalizeQueuedRun(ctx, run, revision, RunStatus(step.text("status", "completed")), result, errorCode, now)
	case "checkpoint":
		_, err = repository.CompareAndSwapCheckpoint(ctx, workflow, database, step.optionalInteger("checkpoint_revision"), checkpoint)
	case "finalize_checkpoint":
		_, err = repository.FinalizeRunWithCheckpoint(ctx, run, owner, revision, RunStatus(step.text("status", "completed")), result, errorCode, workflow, database, step.optionalInteger("checkpoint_revision"), checkpoint, step.integer("lease_revision", 0))
	case "items", "insert_items":
		var raw []durableStep
		json.Unmarshal(step["items"], &raw)
		items := []RunItemCreate{}
		for _, entry := range raw {
			items = append(items, RunItemCreate{ItemKind: ItemKind(entry.text("kind", "article")), ItemKey: entry.text("key", "1"), UserId: entry.optionalInteger("user_id"), ArticleId: entry.optionalInteger("article_id")})
		}
		if step.text("op", "") == "items" {
			_, err = repository.EnsureRunItems(ctx, run, items, now)
		} else {
			_, err = repository.InsertRunItems(ctx, run, items, now)
		}
	case "claim_item", "claim_next":
		var record *RunItemRecord
		if step.text("op", "") == "claim_item" {
			record, err = repository.ClaimItem(ctx, run, owner, revision, item, step.text("item_owner", owner), now, seconds)
		} else {
			record, err = repository.ClaimNextItem(ctx, run, owner, revision, step.text("item_owner", owner), now, seconds)
		}
		if err != nil || record == nil {
			return nil, err
		}
		return map[string]any{"id": record.Id}, nil
	case "sending":
		_, err = repository.MarkItemSending(ctx, item, owner, revision, now)
	case "finalize_item":
		_, err = repository.FinalizeItem(ctx, item, owner, revision, ItemStatus(step.text("status", "succeeded")), result, errorCode, now)
	case "reserve":
		outcome, err := repository.ReserveDedupe(ctx, workflow, database, step.integer("user_id", 1), step.integer("article_id", 7), run, owner, now)
		if err != nil {
			return nil, err
		}
		return map[string]any{"kind": outcome.Kind, "id": outcome.Record.Id}, nil
	case "resolve":
		_, err = repository.ResolveDedupe(ctx, step.integer("dedupe", 1), run, owner, revision, DedupeStatus(step.text("dedupe_status", "confirmed")), message, now)
	case "release_reservations":
		return repository.ReleaseReservations(ctx, run, owner, reservations)
	case "finalize_attempt":
		_, err = repository.FinalizeAttempt(ctx, item, owner, revision, ItemStatus(step.text("status", "succeeded")), result, errorCode, run, reservations, DedupeStatus(step.text("dedupe_status", "confirmed")), message, now)
	case "cleanup":
		return repository.CleanupConfirmedDedupe(ctx, workflow, database, now)
	case "acquire":
		outcome, err := repository.AcquireLease(ctx, workflow, database, run, owner, now, seconds)
		if err != nil {
			return nil, err
		}
		return map[string]any{"kind": outcome.Kind, "id": outcome.Lease.Id}, nil
	case "renew_lease":
		_, err = repository.RenewLease(ctx, workflow, database, run, owner, revision, now, seconds)
	case "release_lease":
		_, err = repository.ReleaseLease(ctx, workflow, database, run, owner, revision, now)
	case "reconcile":
		return repository.ReconcileAfterTakeover(ctx, run, owner, revision, now)
	case "load":
		record, err := repository.LoadRun(ctx, run)
		if err != nil || record == nil {
			return nil, err
		}
		return map[string]any{"id": record.Id}, nil
	case "list_items":
		items, err := repository.ListRunItems(ctx, run)
		return len(items), err
	case "load_checkpoint":
		record, err := repository.LoadCheckpoint(ctx, workflow, database)
		if err != nil || record == nil {
			return nil, err
		}
		return map[string]any{"id": record.Id}, nil
	case "load_lease":
		record, err := repository.LoadLease(ctx, workflow, database)
		if err != nil || record == nil {
			return nil, err
		}
		return map[string]any{"id": record.Id}, nil
	case "noop":
	default:
		panic("unknown durable fixture operation")
	}
	return "ok", err
}

func TestOriginalRustDurableHistories(t *testing.T) {
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
	for _, history := range corpus.Cases {
		t.Run(history.Name, func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "auth.sqlite")
			ctx := context.Background()
			if _, err := migration.Migrate(ctx, filename); err != nil {
				t.Fatal(err)
			}
			repository, err := Open(filename)
			if err != nil {
				t.Fatal(err)
			}
			defer repository.Close()
			if _, err = repository.database.Exec(`INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'fixture','hash','salt',1,1),(2,'second','hash','salt',1,1)`); err != nil {
				t.Fatal(err)
			}
			for index, step := range history.Steps {
				if statement := step.text("sql", ""); statement != "" {
					if _, err = repository.database.Exec(statement); err != nil {
						t.Fatal(err)
					}
				}
				outcome, err := performDurableStep(ctx, repository, step)
				if err != nil {
					outcome = map[string]any{"error": err.Error()}
				}
				observation := map[string]any{"outcome": outcome, "tables": snapshotDelivery(t, repository)}
				encoded, err := json.Marshal(observation)
				if err != nil {
					t.Fatal(err)
				}
				var actual any
				if err = json.Unmarshal(encoded, &actual); err != nil {
					t.Fatal(err)
				}
				if !reflect.DeepEqual(actual, history.Output[index]) {
					expected, _ := json.Marshal(history.Output[index])
					t.Fatalf("step %d (%s): got %s\nwant %s", index, step.text("op", ""), encoded, expected)
				}
			}
		})
	}
}
