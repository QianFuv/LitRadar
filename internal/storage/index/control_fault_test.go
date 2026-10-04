package index

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func frozenContentInput(t *testing.T) (domain.JournalCatalogEntry, domain.ProviderBatch) {
	t.Helper()
	body, err := os.ReadFile("../../../tests/migration/index/content-vectors.json")
	if err != nil {
		t.Fatal(err)
	}
	var corpus struct {
		Observations []struct {
			Input struct {
				Name       string
				Operations []struct {
					Catalog domain.JournalCatalogEntry
					Batch   domain.ProviderBatch
				}
			}
		}
	}
	if err := json.Unmarshal(body, &corpus); err != nil {
		t.Fatal(err)
	}
	for _, item := range corpus.Observations {
		if item.Input.Name == "first-and-replay" {
			return item.Input.Operations[0].Catalog, item.Input.Operations[0].Batch
		}
	}
	t.Fatal("missing independent fixture")
	return domain.JournalCatalogEntry{}, domain.ProviderBatch{}
}

func preparedControl(t *testing.T) (*Connection, SyncRun, string) {
	t.Helper()
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "control.sqlite")
	control, err := OpenControl(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { control.Close() })
	run := SyncRun{Scope: SyncScope{"catalog", "scholarly", "catalog:example"}, BatchId: "batch-1", RunId: "run-1", Mode: domain.Incremental}
	if _, err := PrepareJournalSync(ctx, control.Conn, run, true, "epoch"); err != nil {
		t.Fatal(err)
	}
	var now int64
	if err := control.QueryRowContext(ctx, "SELECT unixepoch()").Scan(&now); err != nil {
		t.Fatal(err)
	}
	if err := AcquireLease(ctx, control.Conn, run.Scope.CatalogName, run.Scope.ProviderName, run.RunId, now); err != nil {
		t.Fatal(err)
	}
	return control, run, path
}

func TestContentCommitSurvivesProgressFailureAndReplay(t *testing.T) {
	ctx := context.Background()
	control, run, _ := preparedControl(t)
	content, err := OpenContent(ctx, filepath.Join(t.TempDir(), "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer content.Close()
	catalog, batch := frozenContentInput(t)
	if _, err := control.ExecContext(ctx, `CREATE TRIGGER fail_progress BEFORE INSERT ON provider_sync_anchors BEGIN SELECT RAISE(ABORT,'injected control'); END`); err != nil {
		t.Fatal(err)
	}
	write := func() (ContentWriteOutcome, error) {
		return WriteContentBatch(ctx, content.Conn, catalog, batch, "stable-revision", "epoch")
	}
	progress := domain.ProviderProgress{State: domain.Complete, NextAnchor: ptr("opaque-success")}
	_, err = CommitContentThenProgress(ctx, control.Conn, run, progress, "done", write)
	var failure *ContentCheckpointError
	if !errors.As(err, &failure) || failure.Phase != "control" || !strings.HasPrefix(err.Error(), "sync progress commit failed: ") {
		t.Fatalf("error=%v", err)
	}
	for _, table := range []string{"articles", "article_change_events"} {
		var count int
		if err := content.QueryRowContext(ctx, "SELECT COUNT(*) FROM "+table).Scan(&count); err != nil || count != 1 {
			t.Fatalf("%s count=%d err=%v", table, count, err)
		}
	}
	anchor, err := ReadSyncAnchor(ctx, control.Conn, run.Scope)
	if err != nil || anchor != nil {
		t.Fatalf("premature anchor=%v err=%v", anchor, err)
	}
	checkpoint, err := ReadRunCheckpoint(ctx, control.Conn, run.Scope)
	if err != nil || checkpoint == nil || checkpoint.TraversalCheckpoint != nil {
		t.Fatalf("lost checkpoint=%v err=%v", checkpoint, err)
	}
	if _, err := control.ExecContext(ctx, "DROP TRIGGER fail_progress"); err != nil {
		t.Fatal(err)
	}
	outcome, err := CommitContentThenProgress(ctx, control.Conn, run, progress, "done", write)
	if err != nil {
		t.Fatal(err)
	}
	if outcome.ArticlesChanged != 0 || outcome.IdentityAliasesAdded != 0 || outcome.ChangeEventsEmitted != 0 {
		t.Fatalf("duplicated replay: %+v", outcome)
	}
	anchor, err = ReadSyncAnchor(ctx, control.Conn, run.Scope)
	if err != nil || anchor == nil || anchor.CommittedAnchor == nil || *anchor.CommittedAnchor != "opaque-success" {
		t.Fatalf("anchor=%v err=%v", anchor, err)
	}
	checkpoint, err = ReadRunCheckpoint(ctx, control.Conn, run.Scope)
	if err != nil || checkpoint != nil {
		t.Fatalf("checkpoint=%v err=%v", checkpoint, err)
	}
}

func TestContentFailureAndOwnershipFenceDoNotAdvance(t *testing.T) {
	for _, scenario := range []string{"content", "expired", "stale-run", "stale-base", "stale-batch"} {
		t.Run(scenario, func(t *testing.T) {
			ctx := context.Background()
			control, run, _ := preparedControl(t)
			cause := errors.New("injected content")
			called := false
			switch scenario {
			case "expired":
				if _, err := control.ExecContext(ctx, "UPDATE provider_leases SET expires_at=unixepoch()"); err != nil {
					t.Fatal(err)
				}
			case "stale-run":
				run.RunId = "other"
			case "stale-base":
				run.BaseAnchor = ptr("wrong")
			case "stale-batch":
				run.BatchId = "other"
			}
			_, err := CommitContentThenProgress(ctx, control.Conn, run, domain.ProviderProgress{State: domain.Continue, Checkpoint: ptr("next")}, "later", func() (int, error) { called = true; return 0, cause })
			if err == nil {
				t.Fatal("expected failure")
			}
			if called != (scenario == "content") {
				t.Fatalf("content callback called=%t", called)
			}
			var typed *ContentCheckpointError
			if !errors.As(err, &typed) {
				t.Fatal(err)
			}
			expectedPhase := "control"
			if scenario == "content" {
				expectedPhase = "content"
				if !errors.Is(err, cause) {
					t.Fatal(err)
				}
			}
			if typed.Phase != expectedPhase {
				t.Fatal(typed)
			}
			checkpoint, err := ReadRunCheckpoint(ctx, control.Conn, run.Scope)
			if err != nil || checkpoint == nil || checkpoint.TraversalCheckpoint != nil {
				t.Fatalf("progress advanced: %v %v", checkpoint, err)
			}
		})
	}
}

func TestControlLockFencesTakeoverAcrossContentCommit(t *testing.T) {
	ctx := context.Background()
	control, run, path := preparedControl(t)
	contender, err := OpenControl(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer contender.Close()
	started := make(chan struct{})
	acquired := make(chan error, 1)
	_, err = CommitContentThenProgress(ctx, control.Conn, run, domain.ProviderProgress{State: domain.Continue, Checkpoint: ptr("next")}, "later", func() (int, error) {
		go func() {
			close(started)
			acquired <- AcquireLease(ctx, contender.Conn, run.Scope.CatalogName, run.Scope.ProviderName, "contender", time.Now().Unix()+1000)
		}()
		<-started
		select {
		case err := <-acquired:
			return 0, fmt.Errorf("takeover escaped control fence: %v", err)
		case <-time.After(100 * time.Millisecond):
			return 1, nil
		}
	})
	if err != nil {
		t.Fatal(err)
	}
	select {
	case err := <-acquired:
		if err != nil {
			t.Fatal(err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("takeover did not proceed after commit")
	}
	checkpoint, err := ReadRunCheckpoint(ctx, control.Conn, run.Scope)
	if err != nil || checkpoint == nil || checkpoint.TraversalCheckpoint == nil || *checkpoint.TraversalCheckpoint != "next" {
		t.Fatalf("checkpoint=%v err=%v", checkpoint, err)
	}
}

func TestImmediateTransactionRollsBackPanic(t *testing.T) {
	ctx := context.Background()
	control, _, _ := preparedControl(t)
	func() {
		defer func() {
			if recover() == nil {
				t.Fatal("missing panic")
			}
		}()
		_ = immediate(ctx, control.Conn, func() error {
			if _, err := control.ExecContext(ctx, "DELETE FROM provider_leases"); err != nil {
				t.Fatal(err)
			}
			panic("injected")
		})
	}()
	var count int
	if err := control.QueryRowContext(ctx, "SELECT COUNT(*) FROM provider_leases").Scan(&count); err != nil || count != 1 {
		t.Fatalf("panic leaked transaction: count=%d err=%v", count, err)
	}
	if err := immediate(ctx, control.Conn, func() error { return nil }); err != nil {
		t.Fatal(err)
	}
}

func TestOpaqueControlFormattingDoesNotExposeState(t *testing.T) {
	secret := "private-provider-cookie"
	anchor := ProviderSyncAnchor{CommittedAnchor: &secret}
	checkpoint := ProviderRunCheckpoint{BaseAnchor: &secret, TraversalCheckpoint: &secret}
	run := SyncRun{BaseAnchor: &secret}
	for _, value := range []any{anchor, &anchor, checkpoint, &checkpoint, run, &run} {
		for _, format := range []string{"%v", "%+v", "%#v", "%s"} {
			if text := fmt.Sprintf(format, value); strings.Contains(text, secret) {
				t.Fatalf("leaked opaque state: %s", text)
			}
		}
	}
}

func TestContentRuntimeChecksStructureWithoutForeignKeyDataScan(t *testing.T) {
	ctx := context.Background()
	path := filepath.Join(t.TempDir(), "content.sqlite")
	content, err := OpenContent(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := content.ExecContext(ctx, "PRAGMA foreign_keys=OFF; INSERT INTO article_retraction_dois VALUES(123,'10.1/orphan')"); err != nil {
		t.Fatal(err)
	}
	content.Close()
	reopened, err := OpenContent(ctx, path)
	if err != nil {
		t.Fatal(err)
	}
	defer reopened.Close()
	rows, err := reopened.QueryContext(ctx, "PRAGMA foreign_key_check")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	if !rows.Next() {
		t.Fatal("fixture did not retain orphan")
	}
}
