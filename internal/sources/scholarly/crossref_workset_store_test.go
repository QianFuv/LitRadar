package scholarly

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/transport"
)

func worksetTestState() CrossrefCheckpoint {
	return CrossrefCheckpoint{Token: "0123456789abcdef0123456789abcdef", Issn: "1234-5679", FrozenAt: 2, Phase: CrossrefPhase{Kind: "discover"}}
}

func TestCrossrefWorksetFullDiskRollsBackAndConnectionRemainsUsable(t *testing.T) {
	workset := createTestWorkset(t, worksetTestState())
	var pages int
	if err := workset.row("PRAGMA page_count").Scan(&pages); err != nil {
		t.Fatal(err)
	}
	if err := workset.exec(fmt.Sprintf("PRAGMA max_page_count=%d", pages)); err != nil {
		t.Fatal(err)
	}
	page := worksetTestPage(0, 1, 1, 1)
	page.Items[0].(map[string]any)["abstract"] = strings.Repeat("x", 1024*1024)
	before := workset.Checkpoint()
	if _, err := workset.Accept(page); err == nil || !strings.Contains(err.Error(), "full") {
		t.Fatalf("expected SQLite full, got %v", err)
	}
	if !reflect.DeepEqual(before, workset.Checkpoint()) || countWorksetRows(t, workset, "works") != 0 {
		t.Fatal("disk exhaustion leaked transaction")
	}
	if err := workset.exec("PRAGMA max_page_count=1048576"); err != nil {
		t.Fatal(err)
	}
	acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 1))
}

func TestCrossrefWorksetEmissionBudgetPrecedesPayloadAllocation(t *testing.T) {
	workset := createTestWorkset(t, worksetTestState())
	acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 1))
	if err := workset.exec("UPDATE works SET payload=CAST(zeroblob(64*1024*1024) AS TEXT)"); err != nil {
		t.Fatal(err)
	}
	var before, after runtime.MemStats
	runtime.ReadMemStats(&before)
	_, err := workset.Emit("9999-12-31", nil, nil)
	runtime.ReadMemStats(&after)
	if err == nil || err.Error() != "Crossref work exceeds the local emission size limit" {
		t.Fatal(err)
	}
	if after.TotalAlloc-before.TotalAlloc > 16*1024*1024 {
		t.Fatalf("oversized payload copied before budget check: %d", after.TotalAlloc-before.TotalAlloc)
	}
	runtime.ReadMemStats(&before)
	var legacyPayload string
	if err := workset.row("SELECT payload FROM works").Scan(&legacyPayload); err != nil {
		t.Fatal(err)
	}
	runtime.ReadMemStats(&after)
	if len(legacyPayload) != 64*1024*1024 || after.TotalAlloc-before.TotalAlloc < 64*1024*1024 {
		t.Fatal("driver-copy regression control did not exercise old eager read")
	}
}

func TestCrossrefWorksetNegativeVersionAndFileOnlyUnlink(t *testing.T) {
	initial := worksetTestState()
	workset := createTestWorkset(t, initial)
	if err := workset.exec("PRAGMA user_version=-1"); err != nil {
		t.Fatal(err)
	}
	root := workset.root
	workset.Close()
	restored, _, err := OpenCrossrefWorkset(root, "catalog", initial)
	if err == nil {
		restored.Close()
		t.Fatal("negative version accepted")
	}
	failure, ok := err.(*provider.Error)
	if !ok || failure.Kind != provider.Internal {
		t.Fatalf("version storage conversion changed: %v", err)
	}
	directory := filepath.Join(root, "empty-directory")
	if err := os.Mkdir(directory, 0700); err != nil {
		t.Fatal(err)
	}
	if err := unlinkWorksetFile(directory); err == nil {
		t.Fatal("file-only cleanup removed a directory")
	}
	if _, err := os.Stat(directory); err != nil {
		t.Fatal(err)
	}
}

func worksetTestWork(index int, second int64) any {
	work, err := transport.ParseJson([]byte(fmt.Sprintf(`{"DOI":"10.1234/%d","title":["Work %d"],"created":{"timestamp":%d},"published":{"date-parts":[[2026,1,1]]},"volume":"1","issue":"1"}`, index, index, second*1000)))
	if err != nil {
		panic(err)
	}
	return work
}

func worksetTestPage(start, count int, second int64, total uint64) CrossrefPage {
	page := CrossrefPage{TotalResults: total, NextCursor: new(fmt.Sprint(start + count)), Items: []any{}}
	for index := start; index < start+count; index++ {
		page.Items = append(page.Items, worksetTestWork(index, second))
	}
	return page
}

func createTestWorkset(t *testing.T, state CrossrefCheckpoint) *CrossrefWorkset {
	t.Helper()
	workset, err := CreateCrossrefWorkset(t.TempDir(), "catalog", state)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { workset.Close() })
	return workset
}

func acceptTestWorkset(t *testing.T, workset *CrossrefWorkset, page CrossrefPage) CrossrefCheckpoint {
	t.Helper()
	state, err := workset.Accept(page)
	if err != nil {
		t.Fatal(err)
	}
	return state
}

func countWorksetRows(t *testing.T, workset *CrossrefWorkset, table string) int {
	t.Helper()
	var count int
	if err := workset.row("SELECT count(*) FROM " + table).Scan(&count); err != nil {
		t.Fatal(err)
	}
	return count
}

// TestCrossrefWorksetSingletonReplayAndCoreEmission checks singleton collection through exact checkpoint replay and emission.
func TestCrossrefWorksetSingletonReplayAndCoreEmission(t *testing.T) {
	initial := worksetTestState()
	workset := createTestWorkset(t, initial)
	complete := acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 1))
	if complete.Phase.Kind != "ready" || complete.Sequence != 1 || countWorksetRows(t, workset, "works") != 1 {
		t.Fatal(complete)
	}
	root := workset.root
	if err := workset.Close(); err != nil {
		t.Fatal(err)
	}
	workset, replay, err := OpenCrossrefWorkset(root, "catalog", initial)
	if err != nil || !replay || !reflect.DeepEqual(workset.Checkpoint(), complete) {
		t.Fatalf("reopen: %v %v", replay, err)
	}
	defer workset.Close()
	sealed, page := assertSingletonWorksetEmission(t, workset)
	if err := workset.Close(); err != nil {
		t.Fatal(err)
	}
	assertSingletonCoreEmissionReplay(t, root, complete, sealed, page)
}

func TestCrossrefWorksetDenseCursorRequiresTerminalPage(t *testing.T) {
	for _, hasUnexpectedTail := range []bool{false, true} {
		t.Run(fmt.Sprint(hasUnexpectedTail), func(t *testing.T) {
			initial := worksetTestState()
			initial.FrozenAt = 1
			workset := createTestWorkset(t, initial)
			acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 450))
			acceptTestWorkset(t, workset, worksetTestPage(0, 225, 1, 450))
			acceptTestWorkset(t, workset, worksetTestPage(0, 225, 1, 450))
			state := acceptTestWorkset(t, workset, worksetTestPage(225, 225, 1, 450))
			if state.Phase.Kind != "collect" || state.Phase.Received != 450 {
				t.Fatal("full final cursor page was accepted as terminal", state)
			}
			count := 0
			if hasUnexpectedTail {
				count = 1
			}
			state = acceptTestWorkset(t, workset, worksetTestPage(450, count, 1, 450))
			if hasUnexpectedTail {
				if state.Phase.Retry != 1 || state.Phase.Received != 0 || countWorksetRows(t, workset, "works") != 0 {
					t.Fatal(state)
				}
				if _, err := workset.Accept(worksetTestPage(0, 1, 1, 450)); err == nil {
					t.Fatal("second incomplete cursor did not fail")
				}
				if !reflect.DeepEqual(state, workset.Checkpoint()) {
					t.Fatal("failure advanced checkpoint")
				}
			} else {
				if state.Phase.Kind != "ready" {
					t.Fatal(state)
				}
				first, err := workset.Emit("9999-12-31", nil, nil)
				if err != nil || len(first.Works) != 225 || !first.HasMore {
					t.Fatalf("first: %+v %v", first, err)
				}
				last, err := workset.Emit("9999-12-31", nil, first.After)
				if err != nil || len(last.Works) != 225 || last.HasMore {
					t.Fatalf("last: %d %v %v", len(last.Works), last.HasMore, err)
				}
			}
		})
	}
}

func TestCrossrefWorksetRollbackIncludesGroupingAndMetadata(t *testing.T) {
	for _, table := range []string{"works", "groups", "metadata"} {
		t.Run(table, func(t *testing.T) {
			workset := createTestWorkset(t, worksetTestState())
			operation := "INSERT"
			if table == "metadata" {
				operation = "UPDATE"
			}
			if err := workset.exec("CREATE TRIGGER fail_write BEFORE " + operation + " ON " + table + " BEGIN SELECT RAISE(ABORT, 'injected write failure'); END"); err != nil {
				t.Fatal(err)
			}
			before := workset.Checkpoint()
			if _, err := workset.Accept(worksetTestPage(0, 1, 1, 1)); err == nil {
				t.Fatal("injected write failure ignored")
			}
			if !reflect.DeepEqual(before, workset.Checkpoint()) {
				t.Fatal("memory state escaped rollback")
			}
			for _, name := range []string{"works", "groups", "partitions"} {
				if countWorksetRows(t, workset, name) != 0 {
					t.Fatal("transaction leaked " + name)
				}
			}
			if err := workset.exec("DROP TRIGGER fail_write"); err != nil {
				t.Fatal(err)
			}
			acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 1))
		})
	}
	t.Run("sequence overflow", func(t *testing.T) {
		state := worksetTestState()
		state.Sequence = math.MaxUint64
		workset := createTestWorkset(t, state)
		if _, err := workset.Accept(worksetTestPage(0, 1, 1, 1)); err == nil || err.Error() != "Crossref sequence overflow" {
			t.Fatal(err)
		}
		if countWorksetRows(t, workset, "works") != 0 {
			t.Fatal("sequence overflow leaked writes")
		}
	})
}

func TestCrossrefWorksetOwnershipAndDamageBoundary(t *testing.T) {
	for _, mutation := range []string{"state", "owner", "version", "missing metadata", "blob state", "missing database", "missing manifest", "corrupt database"} {
		t.Run(mutation, func(t *testing.T) {
			initial := worksetTestState()
			workset := createTestWorkset(t, initial)
			root := workset.root
			queries := map[string]string{"state": "UPDATE metadata SET state='{}'", "owner": "UPDATE metadata SET owner='{}'", "version": "PRAGMA user_version=2", "missing metadata": "DROP TABLE metadata", "blob state": "UPDATE metadata SET state=x'7b7d'"}
			if query, ok := queries[mutation]; ok {
				if err := workset.exec(query); err != nil {
					t.Fatal(err)
				}
			}
			workset.Close()
			paths, _ := worksetPaths(root, initial.Token)
			switch mutation {
			case "missing database":
				if err := os.Remove(paths[0]); err != nil {
					t.Fatal(err)
				}
			case "missing manifest":
				if err := os.Remove(paths[4]); err != nil {
					t.Fatal(err)
				}
			case "corrupt database":
				if err := os.WriteFile(paths[0], []byte("not a database"), 0600); err != nil {
					t.Fatal(err)
				}
			}
			restored, replay, err := OpenCrossrefWorkset(root, "catalog", initial)
			shouldRebuild := mutation == "state" || mutation == "missing database" || mutation == "corrupt database"
			if shouldRebuild {
				if err != nil || !replay {
					t.Fatalf("rebuild: %v %v", replay, err)
				}
				defer restored.Close()
				if restored.state.Token == initial.Token || restored.state.Sequence != 1 {
					t.Fatal(restored.state)
				}
				if _, err := os.Lstat(paths[4]); !os.IsNotExist(err) {
					t.Fatal("old manifest retained", err)
				}
			} else {
				if err == nil {
					restored.Close()
					t.Fatal("foreign or malformed context was accepted")
				}
				if _, err := os.Lstat(paths[0]); err != nil {
					t.Fatal("rejected database removed", err)
				}
			}
		})
	}
}

func TestCrossrefWorksetPreviousIsExactAndWindowIncludesWholeGroup(t *testing.T) {
	workset := createTestWorkset(t, worksetTestState())
	acceptTestWorkset(t, workset, worksetTestPage(0, 1, 1, 2))
	before := workset.Checkpoint()
	page := worksetTestPage(0, 2, 1, 2)
	page.Items[0].(map[string]any)["published"] = map[string]any{"date-parts": []any{[]any{json.Number("2026"), json.Number("2"), json.Number("1")}}}
	acceptTestWorkset(t, workset, page)
	group, err := workset.FirstGroup()
	if err != nil || group == nil || group.Date != "2026-02-01" {
		t.Fatalf("group: %+v %v", group, err)
	}
	emitted, err := workset.Emit("2026-02-01", new("2026-02-01"), nil)
	if err != nil || len(emitted.Works) != 2 {
		t.Fatal("group window lost older work", len(emitted.Works), err)
	}
	encoded, _ := before.MarshalJSON()
	if err := workset.exec("UPDATE metadata SET previous=?1", strings.Replace(string(encoded), "{", "{ ", 1)); err != nil {
		t.Fatal(err)
	}
	root := workset.root
	workset.Close()
	restored, _, err := OpenCrossrefWorkset(root, "catalog", before)
	if err == nil {
		restored.Close()
		t.Fatal("semantically equivalent previous bytes must not qualify for replay")
	}
}

func TestCrossrefWorksetRejectsPathAliasesAndConcurrentRootCreation(t *testing.T) {
	root := t.TempDir()
	for _, path := range []string{"relative", root + string(filepath.Separator) + ".." + string(filepath.Separator) + "escape"} {
		if _, err := CreateCrossrefWorkset(path, "catalog", worksetTestState()); err == nil {
			t.Fatal("unsafe root accepted", path)
		}
	}
	for index := 0; index < 3; index++ {
		t.Run(fmt.Sprint(index), func(t *testing.T) {
			t.Parallel()
			state := worksetTestState()
			state.Token = fmt.Sprintf("%032x", index+1)
			workset, err := CreateCrossrefWorkset(filepath.Join(root, "shared", "worksets"), "catalog", state)
			if err != nil {
				t.Fatal(err)
			}
			if err := workset.Discard(); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestCrossrefWorksetLargeCollectionsHaveNoJournalSizeCutoff(t *testing.T) {
	for _, isDistributed := range []bool{false, true} {
		t.Run(fmt.Sprint(isDistributed), func(t *testing.T) {
			const count = 100001
			initial := worksetTestState()
			initial.FrozenAt = 1800000000
			initial.UpdatedFrom = new("2026-01-01")
			workset := createTestWorkset(t, initial)
			cursorPages := 0
			for steps := 0; workset.state.Phase.Kind != "ready"; steps++ {
				if steps >= 2000 {
					t.Fatal("unbounded collection loop")
				}
				phase := workset.state.Phase
				page := worksetTestPage(0, 1, 1000000000, count)
				if phase.Kind != "discover" {
					query, err := workset.state.Query()
					if err != nil {
						t.Fatal(err)
					}
					if query.UpdatedFrom == nil || *query.UpdatedFrom != "2026-01-01" || query.UpdatedUntil == nil || *query.UpdatedUntil != initial.FrozenAt {
						t.Fatal("frozen update bounds changed")
					}
					start, end := 0, count
					if isDistributed {
						start = min(count, max(0, int(phase.From-1000000000)))
						end = max(start, min(count, max(0, int(phase.Until-1000000000+1))))
					} else if phase.From > 1000000000 || phase.Until < 1000000000 {
						end = 0
					}
					page = CrossrefPage{TotalResults: uint64(end - start), NextCursor: new("unchanging-cursor")}
					if phase.Cursor != nil {
						cursorPages++
					}
					for index := start + int(phase.Received); index < min(end, start+int(phase.Received)+225); index++ {
						second := int64(1000000000)
						if isDistributed {
							second += int64(index)
						}
						work := worksetTestWork(index, second).(map[string]any)
						work["issue"] = fmt.Sprint(1 + index%12)
						page.Items = append(page.Items, work)
					}
				}
				acceptTestWorkset(t, workset, page)
			}
			if countWorksetRows(t, workset, "works") != count || (cursorPages == 0) != isDistributed {
				t.Fatal("collection count or mode changed")
			}
			seen := make(map[string]bool, count)
			var after *EmissionKey
			for {
				page, err := workset.Emit("9999-12-31", nil, after)
				if err != nil {
					t.Fatal(err)
				}
				for _, work := range page.Works {
					doi := work.(map[string]any)["DOI"].(string)
					if seen[doi] {
						t.Fatal("duplicate emitted work")
					}
					seen[doi] = true
				}
				if !page.HasMore {
					break
				}
				if reflect.DeepEqual(after, page.After) {
					t.Fatal("emission made no progress")
				}
				after = page.After
			}
			if len(seen) != count {
				t.Fatal("emission count", len(seen))
			}
		})
	}
}

func TestCrossrefWorksetRejectsLinkedOwnedFiles(t *testing.T) {
	workset := createTestWorkset(t, worksetTestState())
	root, owner := workset.root, workset.owner
	workset.Close()
	paths, _ := worksetPaths(root, owner.Token)
	if runtime.GOOS != "windows" {
		alias := filepath.Join(t.TempDir(), "hardlink")
		if err := os.Link(paths[0], alias); err != nil {
			t.Fatal(err)
		}
		if err := validateWorksetFiles(root, owner); err == nil || err.Error() != "Crossref workset hard links are not allowed" {
			t.Fatal(err)
		}
		if err := os.Remove(alias); err != nil {
			t.Fatal(err)
		}
	}
	symlink := filepath.Join(t.TempDir(), "linked-root")
	if err := os.Symlink(root, symlink); err != nil {
		t.Skipf("host does not permit creation of symbolic links: %v", err)
	}
	if _, err := prepareWorksetRoot(symlink); err == nil {
		t.Fatal("linked root accepted")
	}
}

// assertSingletonWorksetEmission checks sealed selection and emission leave durable state unchanged.
func assertSingletonWorksetEmission(t *testing.T, workset *CrossrefWorkset) (CrossrefCheckpoint, EmissionPage) {
	t.Helper()
	group, err := workset.FirstGroup()
	if err != nil || group == nil {
		t.Fatalf("group: %v %v", group, err)
	}
	sealed, err := workset.SealSelection("9999-12-31", nil, &group.Anchor)
	if err != nil {
		t.Fatal(err)
	}
	page, err := workset.Emit("9999-12-31", nil, nil)
	if err != nil || len(page.Works) != 1 || page.HasMore || page.After == nil {
		t.Fatalf("emit: %+v %v", page, err)
	}
	if !reflect.DeepEqual(sealed, workset.Checkpoint()) {
		t.Fatal("emit changed durable state")
	}
	return sealed, page
}

// assertSingletonCoreEmissionReplay checks exact seal replay, caller cursor continuation and query-only state.
func assertSingletonCoreEmissionReplay(t *testing.T, root string, complete, sealed CrossrefCheckpoint, page EmissionPage) {
	t.Helper()
	recovered, replay, err := OpenCrossrefWorkset(root, "catalog", complete)
	if err != nil || !replay || !reflect.DeepEqual(recovered.Checkpoint(), sealed) {
		t.Fatalf("seal replay: %v %v", replay, err)
	}
	recovered.Close()
	sealed.Sequence++
	sealed.Phase.After = page.After
	recovered, replay, err = OpenCrossrefWorkset(root, "catalog", sealed)
	if err != nil || replay {
		t.Fatalf("core emission: %v %v", replay, err)
	}
	defer recovered.Close()
	empty, err := recovered.Emit("9999-12-31", nil, page.After)
	if err != nil || len(empty.Works) != 0 || !reflect.DeepEqual(empty.After, page.After) {
		t.Fatalf("empty tail: %+v %v", empty, err)
	}
	if err := recovered.exec("DELETE FROM works"); err == nil {
		t.Fatal("restored emission must be query-only")
	}
}
