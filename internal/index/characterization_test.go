package index

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

// workerDecodeCharacterization exposes pointer omission, positional slots and partial field mutation.
type workerDecodeCharacterization struct {
	Count    uint64   `json:"count"`
	Optional *string  `json:"optional"`
	Values   []uint64 `json:"values"`
	Tail     bool     `json:"tail"`
}

// TestWorkerFieldDecodingCharacterization pins strict known fields and selectively ignored unknowns.
func TestWorkerFieldDecodingCharacterization(t *testing.T) {
	cases := []struct {
		body                string
		defaultTail         int
		shouldIgnoreUnknown bool
		shouldFail          bool
		expected            workerDecodeCharacterization
	}{
		{`{"count":2,"values":[],"tail":true}`, 0, false, false, workerDecodeCharacterization{Count: 2, Values: []uint64{}, Tail: true}},
		{`[2,null,[],true]`, 0, false, false, workerDecodeCharacterization{Count: 2, Values: []uint64{}, Tail: true}},
		{`[2,[]]`, 0, false, true, workerDecodeCharacterization{}},
		{`{"count":2,"values":[]}`, 1, false, false, workerDecodeCharacterization{Count: 2, Values: []uint64{}}},
		{`{"unknown":1,"unknown":2,"count":2,"values":[],"tail":true}`, 0, true, false, workerDecodeCharacterization{Count: 2, Values: []uint64{}, Tail: true}},
		{`{"count":2,"count":3,"values":[],"tail":true}`, 0, true, true, workerDecodeCharacterization{Count: 2}},
		{`{"count":2,"values":[3,"bad"],"tail":true}`, 0, false, true, workerDecodeCharacterization{Count: 2, Values: []uint64{3, 0}}},
		{`{"count":2,"optional":7,"values":[],"tail":true}`, 0, false, true, workerDecodeCharacterization{Count: 2, Optional: new(string)}},
		{`{"count":2,"values":[],"tail":true} trailing`, 0, false, true, workerDecodeCharacterization{}},
	}
	for _, candidate := range cases {
		var actual workerDecodeCharacterization
		err := decodeWorkerFields([]byte(candidate.body), &actual, candidate.defaultTail, candidate.shouldIgnoreUnknown)
		if (err != nil) != candidate.shouldFail || !reflect.DeepEqual(actual, candidate.expected) {
			t.Fatalf("body=%s actual=%+v expected=%+v error=%v", candidate.body, actual, candidate.expected, err)
		}
	}
}

// TestWorkerPublicReceiverRemainsUnchangedAfterRejectedFrame pins atomic receiver replacement.
func TestWorkerPublicReceiverRemainsUnchangedAfterRejectedFrame(t *testing.T) {
	expected := ParentMessage{"committed", 8, 9, 10, 11, 12, true}
	for _, body := range []string{
		`{"type":"committed","protocol_version":8,"worker_id":0,"sequence":0,"journal_ordinal":0,"is_complete":true}`,
		`{"type":"committed","protocol_version":8,"worker_id":0,"sequence":0,"journal_ordinal":0,"page_index":0,"is_complete":true,"worker_id":1}`,
		`["committed",8,0,0,0,0]`,
	} {
		actual := expected
		if json.Unmarshal([]byte(body), &actual) == nil || actual != expected {
			t.Fatalf("rejected frame replaced receiver: %+v body=%s", actual, body)
		}
	}
}

// TestCatalogRowValidationPrecedenceAndPartialState pins shape admission before ordered row fields.
func TestCatalogRowValidationPrecedenceAndPartialState(t *testing.T) {
	row := characterizationCatalogRow()
	delete(row, "title")
	row["unexpected"] = "value"
	_, err := BuildCatalogEntry(row)
	if err == nil || !strings.Contains(err.Error(), `missing=["title"], unexpected=["unexpected"]`) {
		t.Fatalf("shape error=%v", err)
	}
	row = characterizationCatalogRow()
	row["issn"] = "invalid"
	row["eissn"] = "also-invalid"
	actual, err := BuildCatalogEntry(row)
	if err == nil || err.Error() != "issn contains an invalid ISSN" || actual.CatalogId != "journal" || actual.Title != "Journal" {
		t.Fatalf("partial=%+v error=%v", actual, err)
	}
}

// characterizationCatalogRow constructs the exact canonical header without borrowing production normalization.
func characterizationCatalogRow() map[string]string {
	row := map[string]string{}
	for _, column := range catalogColumns {
		row[column] = ""
	}
	row["catalog_id"] = "journal"
	row["title"] = "Journal"
	return row
}

// TestExistingManifestAdmissionCharacterization pins old-run reuse, duplicate fields and required summary shape.
func TestExistingManifestAdmissionCharacterization(t *testing.T) {
	config := LiveConfig{ProjectRoot: t.TempDir()}
	input := storage.CatalogInput{CatalogName: "catalog"}
	intent := storage.ManifestIntent{Path: "manifest.json", RunId: "new-run"}
	path := filepath.Join(config.ProjectRoot, intent.Path)
	for _, candidate := range []struct {
		body       string
		shouldFail bool
	}{
		{`{"run_id":"old-run","generated_at":"old","db_name":"catalog.sqlite","summary":{}}`, false},
		{`{"run_id":"","run_id":"old-run","generated_at":"old","db_name":"catalog.sqlite","summary":{},"unknown":true}`, false},
		{`{"run_id":"old-run","generated_at":"old","db_name":"catalog.sqlite","summary":null}`, true},
		{`{"run_id":"old-run","generated_at":"old","db_name":"wrong.sqlite","summary":{}}`, true},
		{`[]`, true},
		{`null`, true},
	} {
		if err := os.WriteFile(path, []byte(candidate.body), 0600); err != nil {
			t.Fatal(err)
		}
		shouldPublish, err := shouldPublishManifest(config, input, storage.BatchCatalogOutcome{RunId: "new-run"}, intent)
		if shouldPublish || (err != nil) != candidate.shouldFail {
			t.Fatalf("body=%s publish=%t err=%v", candidate.body, shouldPublish, err)
		}
	}
	assertManifestRecoveryBypassesFileAdmission(t, config, input, intent)
}

// assertManifestRecoveryBypassesFileAdmission checks durable intent and outcome pointers bypass existing-file reads.
func assertManifestRecoveryBypassesFileAdmission(t *testing.T, config LiveConfig, input storage.CatalogInput, intent storage.ManifestIntent) {
	t.Helper()
	intent.Path = "."
	intent.ThroughEventId = new(int64(1))
	shouldPublish, err := shouldPublishManifest(config, input, storage.BatchCatalogOutcome{}, intent)
	if err != nil || !shouldPublish {
		t.Fatal(shouldPublish, err)
	}
	intent.ThroughEventId = nil
	shouldPublish, err = shouldPublishManifest(config, input, storage.BatchCatalogOutcome{ManifestPath: new("existing")}, intent)
	if err != nil || !shouldPublish {
		t.Fatal(shouldPublish, err)
	}
}

// TestInlineRepeatedCheckpointRetainsCommittedMetrics pins repeat detection after the second durable page.
func TestInlineRepeatedCheckpointRetainsCommittedMetrics(t *testing.T) {
	writer, request, batch, content, control := parentWriterFixture(t)
	batch.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("repeat")}
	calls := 0
	implementation := workerTestProvider(func(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
		calls++
		result := batch
		result.Articles = []domain.ArticleDraft{batch.Articles[0]}
		if calls == 2 {
			result.Articles[0].Title = "Second durable page"
		}
		return result, nil
	})
	metrics, err := indexEntries(context.Background(), content.Conn, control.Conn, writer.context, []domain.JournalCatalogEntry{request.Assignments[0].Entry}, domain.Incremental, true, implementation)
	if err == nil || err.Error() != "index provider returned a repeated checkpoint" || calls != 2 || metrics.PagesCommitted != 2 || metrics.JournalsSucceeded != 0 {
		t.Fatalf("metrics=%+v calls=%d err=%v", metrics, calls, err)
	}
	assertRepeatedInlinePagePersisted(t, content, control, writer.context.syncRun(request.Assignments[0]).Scope)
}

// assertRepeatedInlinePagePersisted checks both the repeated cursor and the second page's distinct content.
func assertRepeatedInlinePagePersisted(t *testing.T, content, control *storage.Connection, scope storage.SyncScope) {
	t.Helper()
	checkpoint, err := storage.ReadRunCheckpoint(context.Background(), control.Conn, scope)
	if err != nil || checkpoint.TraversalCheckpoint == nil || *checkpoint.TraversalCheckpoint != "repeat" {
		t.Fatalf("checkpoint=%+v err=%v", checkpoint, err)
	}
	var count int
	if err := content.QueryRowContext(context.Background(), "SELECT count(*) FROM articles WHERE title='Second durable page'").Scan(&count); err != nil || count != 1 {
		t.Fatalf("second page count=%d err=%v", count, err)
	}
}

// TestNotificationRecordingErrorPrecedesRunnerFailure pins durable recorder error priority.
func TestNotificationRecordingErrorPrecedesRunnerFailure(t *testing.T) {
	config := liveFixture(t, "alpha")
	config.ShouldNotify = true
	expected := errors.New("notification runner failed")
	run := liveRunner(t, func(_ context.Context, entry domain.JournalCatalogEntry, _ domain.IndexFetchContext) (domain.ProviderBatch, error) {
		return liveBatch(entry), nil
	})
	notify := func(ctx context.Context, _ NotifyProcessConfig, _, _, _ string) (NotifyObservation, error) {
		installNotificationRecordingFailure(t, ctx, config)
		return NotifyObservation{}, expected
	}
	_, err := runLiveIndex(context.Background(), config, run, notify)
	if err == nil || errors.Is(err, expected) || !strings.Contains(err.Error(), "injected notification recording failure") {
		t.Fatalf("error priority=%v", err)
	}
}

// installNotificationRecordingFailure injects a failure only after a durable running attempt exists.
func installNotificationRecordingFailure(t *testing.T, ctx context.Context, config LiveConfig) {
	t.Helper()
	connection, err := storage.OpenBatch(ctx, filepath.Join(config.ProjectRoot, "data", "index-control", storage.BatchDatabaseFilename))
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	_, err = connection.ExecContext(ctx, `CREATE TRIGGER reject_notification_result BEFORE UPDATE OF notify_status ON index_batch_catalogs WHEN NEW.notify_status='failed' BEGIN SELECT RAISE(FAIL,'injected notification recording failure'); END`)
	if err != nil {
		t.Fatal(err)
	}
}
