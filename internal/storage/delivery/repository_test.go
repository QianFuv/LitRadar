package delivery

import (
	"context"
	"errors"
	"path/filepath"
	"sync"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func testRepository(t *testing.T) *Repository {
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
	if _, err := repository.database.Exec(`INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'fixture','hash','salt',1,1)`); err != nil {
		t.Fatal(err)
	}
	return repository
}

func pointer[T any](value T) *T { return &value }

func scheduledRun(id string) RunCreate {
	return RunCreate{ExternalId: id, Workflow: WorkflowNotify, ScopeKey: "fixture.sqlite", DbName: pointer("fixture.sqlite"), TriggerKind: TriggerKindScheduled, Mode: RunModeExecute, CreatedAt: 10}
}

func TestCheckpointCasHasOneWinnerAndPreservesRawJson(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	update := CheckpointUpdate{Status: CheckpointStatusCompleted, SnapshotJson: "  [1, true]  ", UpdatedAt: 10.25}
	initial, err := repository.CompareAndSwapCheckpoint(ctx, WorkflowNotify, "fixture.sqlite", nil, update)
	if err != nil {
		t.Fatal(err)
	}
	if initial.Revision != 0 || initial.SnapshotJson != update.SnapshotJson {
		t.Fatalf("initial: %+v", initial)
	}
	var group sync.WaitGroup
	outcomes := make(chan error, 2)
	for range 2 {
		group.Add(1)
		go func() {
			defer group.Done()
			_, err := repository.CompareAndSwapCheckpoint(ctx, WorkflowNotify, "fixture.sqlite", pointer(int64(0)), update)
			outcomes <- err
		}()
	}
	group.Wait()
	close(outcomes)
	winners, conflicts := 0, 0
	for err := range outcomes {
		if err == nil {
			winners++
		} else if errors.Is(err, ErrConflict) {
			conflicts++
		} else {
			t.Fatal(err)
		}
	}
	if winners != 1 || conflicts != 1 {
		t.Fatalf("winners=%d conflicts=%d", winners, conflicts)
	}
}

func TestRunLeaseExpirationAndCompetingScope(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	first, err := repository.EnqueueRun(ctx, scheduledRun("first"))
	if err != nil {
		t.Fatal(err)
	}
	second, err := repository.EnqueueRun(ctx, scheduledRun("second"))
	if err != nil {
		t.Fatal(err)
	}
	claimed, err := repository.ClaimRun(ctx, first.Id, "owner", 0, 10.5, 0.25)
	if err != nil || claimed.Kind != "claimed" {
		t.Fatalf("claim: %+v %v", claimed, err)
	}
	assertRunLeaseExpiryAndCompetition(t, repository, ctx, first, second, claimed)
	finished, err := repository.FinalizeRun(ctx, first.Id, "owner", claimed.Run.Revision, RunStatusCompleted, nil, nil, 12)
	if err != nil || finished.OwnerId != nil || finished.Revision != 2 {
		t.Fatalf("expired untouched owner may finalize: %+v %v", finished, err)
	}
	claimed, err = repository.ClaimRun(ctx, second.Id, "other", 0, 12, 1)
	if err != nil || claimed.Kind != "claimed" {
		t.Fatalf("released competitor: %+v %v", claimed, err)
	}
}

func TestWorkflowLeaseRetainsMonotonicRevision(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	run, err := repository.EnqueueRun(ctx, scheduledRun("lease"))
	if err != nil {
		t.Fatal(err)
	}
	acquired, err := repository.AcquireLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "owner", 10, 1)
	if err != nil || acquired.Kind != "acquired" {
		t.Fatalf("acquire: %+v %v", acquired, err)
	}
	busy, err := repository.AcquireLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "owner", 10.5, 1)
	if err != nil || busy.Kind != "busy" {
		t.Fatalf("same owner busy: %+v %v", busy, err)
	}
	taken, err := repository.AcquireLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "new", 11, 1)
	if err != nil || taken.Lease.Revision != 1 {
		t.Fatalf("takeover: %+v %v", taken, err)
	}
	assertWorkflowLeaseReleaseAndReuse(t, repository, ctx, run)

}

func TestUnknownManualAcknowledgmentRequiresAtomicAudit(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	create := scheduledRun("manual-old")
	create.TriggerKind = TriggerKindManual
	create.UserId = pointer(int64(1))
	admitted, err := repository.AdmitManualRun(ctx, create)
	if err != nil {
		t.Fatal(err)
	}
	claimed, err := repository.ClaimRun(ctx, admitted.Run.Id, "owner", 0, 10, 1)
	if err != nil {
		t.Fatal(err)
	}
	old, err := repository.FinalizeRun(ctx, claimed.Run.Id, "owner", claimed.Run.Revision, RunStatusUnknown, nil, nil, 12)
	if err != nil {
		t.Fatal(err)
	}
	create.ExternalId = "manual-new"
	blocked, err := repository.AdmitManualRun(ctx, create)
	if err != nil || blocked.Kind != "blocked_unknown" {
		t.Fatalf("quarantine: %+v %v", blocked, err)
	}
	assertMissingAcknowledgmentAuditRollsBack(t, repository, ctx, create)
	audit := domain.AuditEvent{Action: "manual_ack", Outcome: "completed", OccurredAt: 13}
	replacement, err := repository.AcknowledgeUnknownManualRun(ctx, 1, "manual-old", create, &audit)
	if err != nil || replacement.Status != RunStatusQueued {
		t.Fatalf("acknowledge: %+v %v", replacement, err)
	}
	assertAcknowledgmentPreservesUnknownAndRejectsReplay(t, repository, ctx, old, create, audit)

}

func TestTakeoverQuarantinesSendingAndResetsSafeClaims(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	run, err := repository.EnqueueRun(ctx, scheduledRun("recovery"))
	if err != nil {
		t.Fatal(err)
	}
	items, err := repository.InsertRunItems(ctx, run.Id, []RunItemCreate{{ItemKind: ItemKindSubscriber, ItemKey: "1", UserId: pointer(int64(1))}, {ItemKind: ItemKindArticle, ItemKey: "2", ArticleId: pointer(int64(2))}}, 10)
	if err != nil {
		t.Fatal(err)
	}
	claimed, err := repository.ClaimRun(ctx, run.Id, "old", 0, 10, 1)
	if err != nil {
		t.Fatal(err)
	}
	sending, reserved := prepareAmbiguousTakeoverItems(t, repository, ctx, run, items, claimed)
	replacement, err := repository.ClaimRun(ctx, run.Id, "new", claimed.Run.Revision, 11, 5)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.ClaimItem(ctx, run.Id, "new", replacement.Run.Revision, sending.Id, "new-item", 11, 1); !errors.Is(err, ErrConflict) {
		t.Fatalf("must not reclaim sending: %v", err)
	}
	recovered, err := repository.ReconcileAfterTakeover(ctx, run.Id, "new", replacement.Run.Revision, 11)
	if err != nil || recovered != (RecoveryResult{ResetItemCount: 1, UnknownItemCount: 1, ReleasedDedupeCount: 1, UnknownDedupeCount: 1}) {
		t.Fatalf("recovery: %+v %v", recovered, err)
	}
	assertTakeoverQuarantineAndSafeRetry(t, repository, ctx, run, items, replacement, reserved)

}

func TestFinalizationRollsBackEarlierCrossTableMutations(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	run, err := repository.EnqueueRun(ctx, scheduledRun("atomic"))
	if err != nil {
		t.Fatal(err)
	}
	claimed, lease, item, reservations := prepareAtomicFinalizationRows(t, repository, ctx, run)
	assertAttemptFinalizationRollbackThenSuccess(t, repository, ctx, run, item, reservations)
	update := CheckpointUpdate{Status: CheckpointStatusCompleted, SnapshotJson: "{}", UpdatedAt: 21}
	if _, err := repository.FinalizeRunWithCheckpoint(ctx, run.Id, "owner", claimed.Run.Revision, RunStatusCompleted, nil, nil, WorkflowNotify, "fixture.sqlite", nil, update, lease.Lease.Revision+1); !errors.Is(err, ErrConflict) {
		t.Fatalf("stale lease should abort checkpoint and run: %v", err)
	}
	assertRunCheckpointRollback(t, repository, ctx, run, claimed)
	completed, err := repository.FinalizeRunWithCheckpoint(ctx, run.Id, "owner", claimed.Run.Revision, RunStatusCompleted, nil, nil, WorkflowNotify, "fixture.sqlite", nil, update, lease.Lease.Revision)
	if err != nil || completed.Lease.OwnerId != nil || completed.Run.Status != RunStatusCompleted || completed.Checkpoint.Revision != 0 {
		t.Fatalf("atomic completion: %+v %v", completed, err)
	}
}

func TestPostWriteReadFailureRollsBackTheTransaction(t *testing.T) {
	for _, target := range []string{"item", "checkpoint"} {
		t.Run(target, func(t *testing.T) {
			repository := testRepository(t)
			ctx := context.Background()
			run, err := repository.EnqueueRun(ctx, scheduledRun("readback"))
			if err != nil {
				t.Fatal(err)
			}
			items, err := repository.InsertRunItems(ctx, run.Id, []RunItemCreate{{ItemKind: ItemKindSubscriber, ItemKey: "1", UserId: pointer(int64(1))}}, 10)
			if err != nil {
				t.Fatal(err)
			}
			claimed, err := repository.ClaimRun(ctx, run.Id, "owner", 0, 10, 10)
			if err != nil {
				t.Fatal(err)
			}
			if target == "item" {
				item, err := repository.ClaimItem(ctx, run.Id, "owner", claimed.Run.Revision, items[0].Id, "owner", 10, 10)
				if err != nil {
					t.Fatal(err)
				}
				reserved, err := repository.ReserveDedupe(ctx, WorkflowNotify, "fixture.sqlite", 1, 11, run.Id, "owner", 10)
				if err != nil {
					t.Fatal(err)
				}
				if _, err := repository.database.Exec(`CREATE TRIGGER remove_final_item AFTER UPDATE ON delivery_run_items WHEN NEW.status='succeeded' BEGIN DELETE FROM delivery_run_items WHERE id=NEW.id; END`); err != nil {
					t.Fatal(err)
				}
				_, err = repository.FinalizeAttempt(ctx, item.Id, "owner", item.Revision, ItemStatusSucceeded, nil, nil, run.Id, []DedupeResolution{{Id: reserved.Record.Id, ExpectedRevision: 0}}, DedupeStatusConfirmed, nil, 11)
				if !errors.Is(err, ErrNotFound) {
					t.Fatalf("missing final item must abort: %v", err)
				}
				dedupe, err := repository.LoadDedupe(ctx, WorkflowNotify, "fixture.sqlite", 1, 11)
				if err != nil || dedupe.Status != DedupeStatusReserved {
					t.Fatalf("rollback: %+v %v", dedupe, err)
				}
			} else {
				lease, err := repository.AcquireLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "owner", 10, 10)
				if err != nil {
					t.Fatal(err)
				}
				if _, err := repository.database.Exec(`CREATE TRIGGER remove_final_checkpoint AFTER UPDATE ON delivery_runs WHEN NEW.status='completed' BEGIN DELETE FROM delivery_checkpoints; END`); err != nil {
					t.Fatal(err)
				}
				_, err = repository.FinalizeRunWithCheckpoint(ctx, run.Id, "owner", claimed.Run.Revision, RunStatusCompleted, nil, nil, WorkflowNotify, "fixture.sqlite", nil, CheckpointUpdate{Status: CheckpointStatusCompleted, SnapshotJson: "{}", UpdatedAt: 11}, lease.Lease.Revision)
				if !errors.Is(err, ErrNotFound) {
					t.Fatalf("missing final checkpoint must abort: %v", err)
				}
				current, err := repository.LoadRun(ctx, run.Id)
				if err != nil || current.Status != RunStatusClaimed {
					t.Fatalf("rollback: %+v %v", current, err)
				}
			}
		})
	}
}

func TestCorruptEnumRemainsRedactedAndNextClaimOverwritesOldOwner(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	run, err := repository.EnqueueRun(ctx, scheduledRun("typed"))
	if err != nil {
		t.Fatal(err)
	}
	claimed, err := repository.ClaimRun(ctx, run.Id, "owner", 0, 10, 10)
	if err != nil {
		t.Fatal(err)
	}
	items, err := repository.InsertRunItems(ctx, run.Id, []RunItemCreate{{ItemKind: ItemKindArticle, ItemKey: "1", ArticleId: pointer(int64(1))}}, 10)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.database.Exec(`UPDATE delivery_run_items SET status='claimed',owner_id=x'aa',lease_expires_at=10 WHERE id=?`, items[0].Id); err != nil {
		t.Fatal(err)
	}
	item, err := repository.ClaimNextItem(ctx, run.Id, "owner", claimed.Run.Revision, "replacement", 11, 1)
	if err != nil || item == nil || item.OwnerId == nil || *item.OwnerId != "replacement" {
		t.Fatalf("next claim must replace old owner before reading: %+v %v", item, err)
	}
	assertStoredRunEnumIsRedacted(t, repository, ctx, run)

}

func assertRunLeaseExpiryAndCompetition(t *testing.T, repository *Repository, ctx context.Context, first, second *RunRecord, claimed RunOutcome) {
	t.Helper()
	busy, err := repository.ClaimRun(ctx, second.Id, "other", 0, 11, 5)
	if err != nil || busy.Kind != "busy" || busy.Run.Id != first.Id {
		t.Fatalf("expired competitor remains busy: %+v %v", busy, err)
	}
	if _, err := repository.StartRun(ctx, first.Id, "owner", claimed.Run.Revision, 10.75); !errors.Is(err, ErrConflict) {
		t.Fatalf("start at expiry: %v", err)
	}
	if _, err := repository.RenewRun(ctx, first.Id, "owner", claimed.Run.Revision, 10.75, 1); !errors.Is(err, ErrConflict) {
		t.Fatalf("renew at expiry: %v", err)
	}
}

func assertWorkflowLeaseReleaseAndReuse(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord) {
	t.Helper()
	if _, err := repository.ReleaseLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "owner", 0, 11); !errors.Is(err, ErrConflict) {
		t.Fatalf("stale release: %v", err)
	}
	released, err := repository.ReleaseLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "new", 1, 15)
	if err != nil || released.Revision != 2 || released.OwnerId != nil {
		t.Fatalf("expired release: %+v %v", released, err)
	}
	again, err := repository.AcquireLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "next", 15, 1)
	if err != nil || again.Lease.Revision != 3 || again.Lease.Id != released.Id {
		t.Fatalf("monotonic reuse: %+v %v", again, err)
	}
}

func assertMissingAcknowledgmentAuditRollsBack(t *testing.T, repository *Repository, ctx context.Context, create RunCreate) {
	t.Helper()
	if _, err := repository.AcknowledgeUnknownManualRun(ctx, 1, "manual-old", create, nil); err == nil || err.Error() != "Security audit persistence failed" {
		t.Fatalf("missing mandatory audit: %v", err)
	}
	var count int
	if err := repository.database.QueryRow("SELECT count(*) FROM delivery_runs").Scan(&count); err != nil || count != 1 {
		t.Fatalf("rollback count=%d error=%v", count, err)
	}
}

func assertAcknowledgmentPreservesUnknownAndRejectsReplay(t *testing.T, repository *Repository, ctx context.Context, old *RunRecord, create RunCreate, audit domain.AuditEvent) {
	t.Helper()
	preserved, err := repository.LoadRun(ctx, old.Id)
	if err != nil || preserved.Status != RunStatusUnknown || preserved.Revision != old.Revision {
		t.Fatalf("old ambiguity must survive: %+v %v", preserved, err)
	}
	create.ExternalId = "manual-third"
	if _, err := repository.AcknowledgeUnknownManualRun(ctx, 1, "manual-old", create, &audit); !errors.Is(err, ErrConflict) {
		t.Fatalf("ack replay: %v", err)
	}
}

func prepareAmbiguousTakeoverItems(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord, items []RunItemRecord, claimed RunOutcome) (*RunItemRecord, DedupeOutcome) {
	t.Helper()
	sending, err := repository.ClaimItem(ctx, run.Id, "old", claimed.Run.Revision, items[0].Id, "item", 10, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.ClaimItem(ctx, run.Id, "old", claimed.Run.Revision, items[1].Id, "item", 10, 1); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.MarkItemSending(ctx, sending.Id, "item", sending.Revision, 10); err != nil {
		t.Fatal(err)
	}
	reserved, err := repository.ReserveDedupe(ctx, WorkflowNotify, "fixture.sqlite", 1, 11, run.Id, "item", 10)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.database.Exec(`INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(2,'second','hash','salt',1,1)`); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.ReserveDedupe(ctx, WorkflowNotify, "fixture.sqlite", 2, 12, run.Id, "item", 10); err != nil {
		t.Fatal(err)
	}
	return sending, reserved
}

func assertTakeoverQuarantineAndSafeRetry(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord, items []RunItemRecord, replacement RunOutcome, reserved DedupeOutcome) {
	t.Helper()
	unknown, err := repository.LoadDedupe(ctx, WorkflowNotify, "fixture.sqlite", 1, 11)
	if err != nil || unknown.Status != DedupeStatusUnknown || unknown.Id != reserved.Record.Id {
		t.Fatalf("ambiguous dedupe: %+v %v", unknown, err)
	}
	assertUnknownDedupeSurvivesRetention(t, repository, ctx)
	retry, err := repository.ClaimNextItem(ctx, run.Id, "new", replacement.Run.Revision, "new-item", 11, 1)
	if err != nil || retry.Id != items[1].Id || retry.AttemptCount != 2 {
		t.Fatalf("safe retry: %+v %v", retry, err)
	}
}

func assertUnknownDedupeSurvivesRetention(t *testing.T, repository *Repository, ctx context.Context) {
	t.Helper()
	if count, err := repository.CleanupConfirmedDedupe(ctx, WorkflowNotify, "fixture.sqlite", 100); err != nil || count != 0 {
		t.Fatalf("retention must preserve unknown: %d %v", count, err)
	}
}

func prepareAtomicFinalizationRows(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord) (RunOutcome, LeaseOutcome, *RunItemRecord, []DedupeResolution) {
	t.Helper()
	items, err := repository.EnsureRunItems(ctx, run.Id, []RunItemCreate{{ItemKind: ItemKindSubscriber, ItemKey: "1", UserId: pointer(int64(1))}}, 10)
	if err != nil {
		t.Fatal(err)
	}
	claimed, err := repository.ClaimRun(ctx, run.Id, "owner", 0, 10, 10)
	if err != nil {
		t.Fatal(err)
	}
	lease, err := repository.AcquireLease(ctx, WorkflowNotify, "fixture.sqlite", run.Id, "owner", 10, 10)
	if err != nil {
		t.Fatal(err)
	}
	item, err := repository.ClaimItem(ctx, run.Id, "owner", claimed.Run.Revision, items[0].Id, "item-owner", 10, 1)
	if err != nil {
		t.Fatal(err)
	}
	dedupe, err := repository.ReserveDedupe(ctx, WorkflowNotify, "fixture.sqlite", 1, 11, run.Id, "item-owner", 10)
	if err != nil {
		t.Fatal(err)
	}
	reservations := []DedupeResolution{{Id: dedupe.Record.Id, ExpectedRevision: 0}}
	return claimed, lease, item, reservations
}

func assertAttemptFinalizationRollbackThenSuccess(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord, item *RunItemRecord, reservations []DedupeResolution) {
	t.Helper()
	if _, err := repository.FinalizeAttempt(ctx, item.Id, "item-owner", item.Revision+1, ItemStatusSucceeded, nil, nil, run.Id, reservations, DedupeStatusConfirmed, pointer("message"), 12); !errors.Is(err, ErrConflict) {
		t.Fatalf("stale item should abort dedupe: %v", err)
	}
	preserved, err := repository.LoadDedupe(ctx, WorkflowNotify, "fixture.sqlite", 1, 11)
	if err != nil || preserved.Status != DedupeStatusReserved || preserved.Revision != 0 {
		t.Fatalf("dedupe rollback: %+v %v", preserved, err)
	}
	if _, err := repository.FinalizeAttempt(ctx, item.Id, "item-owner", item.Revision, ItemStatusSucceeded, nil, nil, run.Id, reservations, DedupeStatusConfirmed, pointer("message"), 12); err != nil {
		t.Fatal(err)
	}
}

func assertRunCheckpointRollback(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord, claimed RunOutcome) {
	t.Helper()
	checkpoint, err := repository.LoadCheckpoint(ctx, WorkflowNotify, "fixture.sqlite")
	if err != nil || checkpoint != nil {
		t.Fatalf("checkpoint rollback: %+v %v", checkpoint, err)
	}
	current, err := repository.LoadRun(ctx, run.Id)
	if err != nil || current.Status != RunStatusClaimed || current.Revision != claimed.Run.Revision {
		t.Fatalf("run rollback: %+v %v", current, err)
	}
}

func assertStoredRunEnumIsRedacted(t *testing.T, repository *Repository, ctx context.Context, run *RunRecord) {
	t.Helper()
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, `PRAGMA ignore_check_constraints=ON`); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(ctx, `UPDATE delivery_runs SET status='corrupt-sentinel' WHERE id=?`, run.Id); err != nil {
		t.Fatal(err)
	}
	_, err = loadRunRecord(ctx, connection, run.Id)
	if err == nil || err.Error() != "Delivery storage operation failed" {
		t.Fatalf("enum diagnostic: %v", err)
	}
}
