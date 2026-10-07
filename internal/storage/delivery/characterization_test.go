package delivery

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"reflect"
	"testing"

	storageconfig "github.com/QianFuv/LitRadar/internal/storage/config"
)

func TestLegacyPrivateDecodeKeepsPartialStateWhilePublicDecodeClearsIt(t *testing.T) {
	cases := []struct {
		raw  string
		want legacyState
	}{
		{`{"db_name":"fixture.sqlite","run":{"run_id":"r","delivered_article_ids":[7,1.0]}}`, legacyState{DbName: "fixture.sqlite", Run: &legacyRun{RunId: "r", DeliveredArticleIds: []int64{7, 0}}}},
		{`{"db_name":"fixture.sqlite","snapshot":{"issue_article_counts":{"1:2":7,"2:3":1.0}}}`, legacyState{DbName: "fixture.sqlite", Snapshot: legacySnapshot{IssueArticleCounts: map[string]int64{"1:2": 7}}}},
	}
	for _, test := range cases {
		var partial legacyState
		err := decodeLegacyValue([]byte(test.raw), reflect.ValueOf(&partial).Elem())
		if err == nil || !reflect.DeepEqual(partial, test.want) {
			t.Fatal("private partial mutation changed", err)
		}
		state, err := decodeLegacy([]byte(test.raw))
		if err == nil || !reflect.DeepEqual(state, legacyState{}) {
			t.Fatal("public decoding exposed partial state", err)
		}
	}
}

func writeLegacyCharacterizationSource(t *testing.T, root, name, body string) string {
	t.Helper()
	directory := filepath.Join(root, "data", "push_state")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(directory, name)
	if err := os.WriteFile(filename, []byte(body), 0600); err != nil {
		t.Fatal(err)
	}
	return filename
}

func TestLegacyMalformedLaterFilePreventsDestinationCreation(t *testing.T) {
	root := t.TempDir()
	filename := writeLegacyCharacterizationSource(t, root, "a.json", `{"db_name":"a.sqlite"}`)
	writeLegacyCharacterizationSource(t, root, "z.json", "{")
	destination := filepath.Join(root, "missing", "auth.sqlite")
	config := storageconfig.FromProjectRoot(root).WithAuthDbPath(destination)
	if _, err := ImportLegacyFiles(context.Background(), config, 100); err == nil {
		t.Fatal("malformed later source accepted")
	}
	if _, err := os.Stat(filepath.Dir(destination)); !os.IsNotExist(err) {
		t.Fatal("destination opened before full source validation", err)
	}
	body, err := os.ReadFile(filename)
	if err != nil || string(body) != `{"db_name":"a.sqlite"}` {
		t.Fatal("source bytes changed", err)
	}
}

func assertLegacyImportTablesEmpty(t *testing.T, repository *Repository) {
	t.Helper()
	for _, table := range []string{"delivery_checkpoints", "delivery_runs", "delivery_run_items", "delivery_dedupe"} {
		var count int
		if err := repository.database.QueryRow("SELECT COUNT(*) FROM " + table).Scan(&count); err != nil {
			t.Fatal(err)
		}
		if count != 0 {
			t.Fatalf("%s retained rolled-back rows", table)
		}
	}
}

func TestLegacyLateCollisionRollsBackEarlierFilesAndCounters(t *testing.T) {
	states := []legacyState{
		{DbName: "second.sqlite", Run: &legacyRun{RunId: "legacy", PendingIssueKeys: []string{"1:2"}, DoneIssueKeys: []string{"1:2"}}},
		{DbName: "second.sqlite", DeliveryDedupe: map[string]string{"+1:7": "one", "1:07": "two"}},
	}
	for _, state := range states {
		repository := testRepository(t)
		if err := validateLegacy("second.json", state); err != nil {
			t.Fatal("fixture rejected before intended late collision", err)
		}
		inputs := []legacyInput{{workflow: WorkflowNotify, sourceName: "first.json", sourceHash: "first", state: legacyState{DbName: "first.sqlite"}}, {workflow: WorkflowNotify, sourceName: "second.json", sourceHash: "second", state: state}}
		result, err := repository.importLegacy(context.Background(), inputs, 100)
		if err == nil || result != (LegacyImportResult{}) {
			t.Fatal("failed scan exposed partial import counters", err)
		}
		assertLegacyImportTablesEmpty(t, repository)
	}
}

func TestLegacyRawHashSkipsIdenticalBytesAndRejectsWhitespaceChange(t *testing.T) {
	repository := testRepository(t)
	root := t.TempDir()
	original := `{"db_name":"fixture.sqlite","status":"completed"}`
	filename := writeLegacyCharacterizationSource(t, root, "fixture.json", original)
	inputs, err := collectLegacy(root)
	if err != nil {
		t.Fatal(err)
	}
	first, err := repository.importLegacy(context.Background(), inputs, 100)
	if err != nil || first.ImportedCount != 1 {
		t.Fatal("initial import failed", err)
	}
	repeated, err := repository.importLegacy(context.Background(), inputs, 101)
	if err != nil || repeated.SkippedCount != 1 {
		t.Fatal("same bytes did not skip", err)
	}
	assertLegacyWhitespaceConflict(t, repository, root, filename, original)
}

func assertLegacyWhitespaceConflict(t *testing.T, repository *Repository, root, filename, original string) {
	t.Helper()
	unchanged, err := os.ReadFile(filename)
	if err != nil || !bytes.Equal(unchanged, []byte(original)) {
		t.Fatal("import rewrote source", err)
	}
	changed := original + " \n"
	if err := os.WriteFile(filename, []byte(changed), 0600); err != nil {
		t.Fatal(err)
	}
	inputs, err := collectLegacy(root)
	if err != nil {
		t.Fatal(err)
	}
	result, err := repository.importLegacy(context.Background(), inputs, 102)
	if err == nil || err.Error() != "Legacy delivery state changed after import" || result != (LegacyImportResult{}) {
		t.Fatal("raw-byte hash conflict changed", err)
	}
}
