package scholarly

import (
	"reflect"
	"strings"
	"testing"
)

// TestCollectionPreparesLaterItemsBeforeAnyInsert preserves validation priority and complete rollback.
func TestCollectionPreparesLaterItemsBeforeAnyInsert(t *testing.T) {
	initial := worksetTestState()
	initial.CreatedFrom = new(int64(1))
	initial.restartCollection()
	workset := createTestWorkset(t, initial)
	if err := workset.exec("CREATE TRIGGER fail_insert BEFORE INSERT ON works BEGIN SELECT RAISE(ABORT,'synthetic insert failure'); END"); err != nil {
		t.Fatal(err)
	}
	page := worksetTestPage(0, 2, 1, 2)
	page.Items[1] = map[string]any{"DOI": "10.1234/invalid"}
	if _, err := workset.Accept(page); err == nil || !strings.Contains(err.Error(), "no numeric creation timestamp") {
		t.Fatalf("later preparation error lost to insertion: %v", err)
	}
	if countWorksetRows(t, workset, "works") != 0 || !reflect.DeepEqual(workset.Checkpoint(), initial) {
		t.Fatal("failed preparation escaped rollback")
	}
}

// TestCollectionInsertFailurePrecedesMissingCursor preserves SQL-error priority before terminal metadata checks.
func TestCollectionInsertFailurePrecedesMissingCursor(t *testing.T) {
	initial := worksetTestState()
	initial.FrozenAt = 1
	initial.CreatedFrom = new(int64(1))
	initial.RootTotal = new(uint64(226))
	initial.Phase = CrossrefPhase{Kind: "collect", Partition: 1, From: 1, Until: 1, Cursor: new("*"), Expected: new(uint64(226))}
	workset := createTestWorkset(t, initial)
	if err := workset.exec("CREATE TRIGGER fail_insert BEFORE INSERT ON works BEGIN SELECT RAISE(ABORT,'synthetic insert failure'); END"); err != nil {
		t.Fatal(err)
	}
	page := worksetTestPage(0, 225, 1, 226)
	page.NextCursor = nil
	if _, err := workset.Accept(page); err == nil || !strings.Contains(err.Error(), "synthetic insert failure") {
		t.Fatalf("insertion error lost to cursor validation: %v", err)
	}
	if countWorksetRows(t, workset, "works") != 0 || !reflect.DeepEqual(workset.Checkpoint(), initial) {
		t.Fatal("failed insertion escaped rollback")
	}
}
