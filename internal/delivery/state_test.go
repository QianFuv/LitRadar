package delivery

import (
	"context"
	"database/sql"
	"errors"
	"path/filepath"
	"testing"

	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/internal/recommend"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func durableFixture(t *testing.T) (*store.Repository, *sql.DB, RunConfig) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := store.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	database, err := platform.Open(platform.Config{Filename: filename, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	if _, err := database.Exec("INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'reader','hash','salt',1,1)"); err != nil {
		t.Fatal(err)
	}
	return repository, database, RunConfig{AuthDbPath: filename, DbName: "fixture.sqlite", Mode: store.RunModeExecute, Workflow: store.WorkflowNotify, Trigger: store.TriggerKindScheduled}
}

func TestDurableAdmissionFinalizesAtomicallyAndReusesTerminalIdentity(t *testing.T) {
	repository, _, config := durableFixture(t)
	ctx := context.Background()
	run, terminal, err := admitDurableRun(ctx, repository, config, nil, "attempt", 100)
	if err != nil || terminal != nil || run == nil || run.run.Status != store.RunStatusRunning {
		t.Fatalf("admission: %v", err)
	}
	if err := run.renew(ctx, repository, 101); err != nil {
		t.Fatal(err)
	}
	if err := run.finalizeWithCheckpoint(ctx, repository, store.RunStatusCompleted, store.CheckpointStatusCompleted, recommend.Snapshot{IssueArticleCounts: map[string]int64{"1:2": 4}}, pointer("completed"), pointer(`{"selected":1}`), nil, 102); err != nil {
		t.Fatal(err)
	}
	if run.lease.OwnerId != nil || run.checkpoint.SnapshotJson != `{"issue_article_counts":{"1:2":4},"inpress_article_counts":{}}` {
		t.Fatalf("atomic checkpoint projection: %s", run.checkpoint.SnapshotJson)
	}
	next, terminal, err := admitDurableRun(ctx, repository, config, nil, "attempt", 103)
	if err != nil || next != nil || terminal == nil || terminal.Id != run.run.Id || terminal.Status != store.RunStatusCompleted {
		t.Fatal("terminal attempt was rerun", err)
	}
}

func TestDurableAdmissionTakesOverExpiredCompetitorAndQuarantinesAmbiguousSend(t *testing.T) {
	repository, _, config := durableFixture(t)
	ctx := context.Background()
	old, _, err := admitDurableRun(ctx, repository, config, nil, "old-attempt", 100)
	if err != nil {
		t.Fatal(err)
	}
	items, err := repository.EnsureRunItems(ctx, old.run.Id, []store.RunItemCreate{{ItemKind: store.ItemKindSubscriber, ItemKey: "1", UserId: pointer(int64(1))}}, 100)
	if err != nil {
		t.Fatal(err)
	}
	claimed, err := repository.ClaimItem(ctx, old.run.Id, old.owner, old.run.Revision, items[0].Id, old.owner, 100, deliveryLeaseSeconds)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.ReserveDedupe(ctx, config.Workflow, config.DbName, 1, 7, old.run.Id, old.owner, 100); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.MarkItemSending(ctx, claimed.Id, old.owner, claimed.Revision, 100); err != nil {
		t.Fatal(err)
	}
	taken, _, err := admitDurableRun(ctx, repository, config, nil, "new-attempt", 3700)
	if err != nil || taken == nil || !taken.didTakeOverCompetingRun || taken.run.Id != old.run.Id {
		t.Fatalf("expired takeover: %v", err)
	}
	items, err = repository.ListRunItems(ctx, old.run.Id)
	if err != nil || items[0].Status != store.ItemStatusUnknown {
		t.Fatalf("ambiguous item retried: %v %v", items, err)
	}
	dedupe, err := repository.LoadDedupe(ctx, config.Workflow, config.DbName, 1, 7)
	if err != nil || dedupe.Status != store.DedupeStatusUnknown {
		t.Fatal("unknown dedupe released", err)
	}
	if err := old.renew(ctx, repository, 3701); !errors.Is(err, store.ErrConflict) {
		t.Fatalf("stale owner renewed: %v", err)
	}
}

func TestDurableAdmissionRejectsLiveCompetitorAndMismatchedRecoveryMode(t *testing.T) {
	repository, _, config := durableFixture(t)
	ctx := context.Background()
	old, _, err := admitDurableRun(ctx, repository, config, nil, "old", 100)
	if err != nil {
		t.Fatal(err)
	}
	if _, _, err := admitDurableRun(ctx, repository, config, nil, "competing", 101); !errors.Is(err, ErrBusy) {
		t.Fatalf("live owner displaced: %v", err)
	}
	config.Mode = store.RunModeDryRun
	if _, _, err := admitDurableRun(ctx, repository, config, nil, "other-mode", 3700); err == nil || err.Error() != "Recovered delivery run does not match the current invocation" {
		t.Fatalf("changed recovery mode accepted: %v", err)
	}
	recovered, err := repository.LoadRun(ctx, old.run.Id)
	if err != nil || recovered.Status != store.RunStatusFailed || recovered.ErrorCode == nil || *recovered.ErrorCode != "recovery_context_mismatch" {
		t.Fatal(recovered, err)
	}
}

func TestBestEffortFailureQuarantinesSendingAndPreservesCheckpoint(t *testing.T) {
	repository, _, config := durableFixture(t)
	ctx := context.Background()
	if _, err := repository.CompareAndSwapCheckpoint(ctx, config.Workflow, config.DbName, nil, store.CheckpointUpdate{Status: store.CheckpointStatusCompleted, SnapshotJson: ` {"prior":true} `, LastCompletedRunAt: pointer("prior"), UpdatedAt: 99}); err != nil {
		t.Fatal(err)
	}
	run, _, err := admitDurableRun(ctx, repository, config, nil, "send", 100)
	if err != nil {
		t.Fatal(err)
	}
	items, err := repository.EnsureRunItems(ctx, run.run.Id, []store.RunItemCreate{{ItemKind: store.ItemKindSubscriber, ItemKey: "1", UserId: pointer(int64(1))}}, 100)
	if err != nil {
		t.Fatal(err)
	}
	item, err := repository.ClaimItem(ctx, run.run.Id, run.owner, run.run.Revision, items[0].Id, run.owner, 100, deliveryLeaseSeconds)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := repository.MarkItemSending(ctx, item.Id, run.owner, item.Revision, 100); err != nil {
		t.Fatal(err)
	}
	run.failBestEffort(ctx, repository, "worker_failed", 101)
	if run.run.Status != store.RunStatusUnknown || run.checkpoint.Status != store.CheckpointStatusUnknown || run.checkpoint.SnapshotJson != ` {"prior":true} ` || run.lease.OwnerId != nil {
		t.Fatal("failed send was treated as safely repeatable")
	}
}
