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

// TestCollectionLateInsertFailureRollsBackPage verifies a reused statement failure leaves no partial work or checkpoint.
func TestCollectionLateInsertFailureRollsBackPage(t *testing.T) {
	workset := createTestWorkset(t, worksetTestState())
	initial := acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 2))
	var before string
	if err := workset.row("SELECT state FROM metadata").Scan(&before); err != nil {
		t.Fatal(err)
	}
	if err := workset.exec("CREATE TRIGGER fail_second_insert BEFORE INSERT ON works WHEN (SELECT count(*) FROM works)=1 BEGIN SELECT RAISE(ABORT,'second insert failure'); END"); err != nil {
		t.Fatal(err)
	}
	page := worksetTestPage(0, 2, 1, 2)
	if _, err := workset.Accept(page); err == nil || !strings.Contains(err.Error(), "second insert failure") {
		t.Fatal("late insert failure was ignored", err)
	}
	var after string
	if err := workset.row("SELECT state FROM metadata").Scan(&after); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(initial, workset.Checkpoint()) || before != after || countWorksetRows(t, workset, "works") != 0 || countWorksetRows(t, workset, "groups") != 0 || countWorksetRows(t, workset, "partitions") != 1 {
		t.Fatal("late failure leaked work or checkpoint state")
	}
	if err := workset.exec("DROP TRIGGER fail_second_insert"); err != nil {
		t.Fatal(err)
	}
	complete := acceptTestWorkset(t, workset, page)
	emitted, err := workset.Emit("9999-12-31", nil, nil)
	if err != nil || complete.Phase.Kind != "ready" || len(emitted.Works) != 2 || emitted.HasMore {
		t.Fatal("same page could not retry after rollback", complete, emitted, err)
	}
}
