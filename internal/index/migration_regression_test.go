package index

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"testing"

	indexdomain "github.com/QianFuv/LitRadar/internal/domain/index"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/platform/process"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func TestFailedBatchMetricsRetainDurableProgressWithoutInventingSuccess(t *testing.T) {
	actual := FailureMetrics(571, 483, 1)
	expected := RunMetrics{JournalsTotal: 571, JournalsSucceeded: 483, JournalsFailed: 1}
	if actual != expected {
		t.Fatal(actual)
	}
	var committed RunMetrics
	committed.record(storage.ContentWriteOutcome{ArticlesSeen: 2, ArticlesChanged: 1, IdentityAliasesAdded: 1, ChangeEventsEmitted: 1})
	if committed != (RunMetrics{PagesCommitted: 1, ArticlesSeen: 2, ArticlesChanged: 1, IdentityAliasesAdded: 1, ChangeEventsEmitted: 1}) {
		t.Fatal(committed)
	}
}

func TestWorkerEnvironmentOverridesInheritedOwner(t *testing.T) {
	t.Setenv(process.ParentEnvironment, "1")
	t.Setenv("LITRADAR_CNKI_CAPTCHA_TOKEN", "private-inherited-token")
	count := 0
	for _, entry := range workerEnvironment() {
		key, value, _ := strings.Cut(entry, "=")
		if strings.EqualFold(key, process.ParentEnvironment) {
			count++
			if key != process.ParentEnvironment || value != strconv.Itoa(os.Getpid()) {
				t.Fatal("inherited owner retained", entry)
			}
		}
		if strings.EqualFold(key, "LITRADAR_CNKI_CAPTCHA_TOKEN") {
			t.Fatal("inherited captcha token retained")
		}
	}
	if count != 1 {
		t.Fatal("worker owner occurrences", count)
	}
}

func TestResumedInlineAndWorkerFetchPreserveFrozenModeAnchorAndCheckpoint(t *testing.T) {
	for _, mode := range []domain.IndexSyncMode{domain.Incremental, domain.FullRescan} {
		t.Run(string(mode), func(t *testing.T) {
			_, request, batch, content, control := parentWriterFixture(t)
			ctx := context.Background()
			assignment := request.Assignments[0]
			oldWriter := WriterContext{"catalog", "cnki", "batch", "run", "epoch"}
			writer := WriterContext{"catalog", "cnki", "next-batch", "run", "epoch"}
			if err := storage.CompleteSyncRun(ctx, control.Conn, oldWriter.syncRun(assignment), testCheckpoint("anchor-old"), "epoch"); err != nil {
				t.Fatal(err)
			}
			assignment.Mode = mode
			prepared, err := storage.PrepareJournalSync(ctx, control.Conn, writer.syncRun(assignment), false, "epoch")
			if err != nil {
				t.Fatal(err)
			}
			assignment.CommittedAnchor = prepared.Checkpoint.BaseAnchor
			if err := storage.AdvanceRunCheckpoint(ctx, control.Conn, writer.syncRun(assignment), "cursor-frozen", "epoch"); err != nil {
				t.Fatal(err)
			}
			requests, metrics, err := PrepareWorkerRequests(ctx, control.Conn, writer, []domain.JournalCatalogEntry{assignment.Entry}, mode, true, indexdomain.Concurrency{WorkerCount: 1, ProcessCount: 1, AggregateCapacity: 1}, 0, 30)
			if err != nil || len(requests) != 1 || metrics.JournalsResumed != 0 {
				t.Fatal(requests, metrics, err)
			}
			var inlineContexts, workerContexts []domain.IndexFetchContext
			makeProvider := func(captured *[]domain.IndexFetchContext) workerTestProvider {
				return func(_ context.Context, _ domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
					*captured = append(*captured, fetch)
					result := batch
					if len(*captured) == 1 {
						result.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("cursor-next")}
					} else {
						result.Progress = domain.ProviderProgress{State: domain.Complete, NextAnchor: testCheckpoint("anchor-new")}
					}
					return result, nil
				}
			}
			if _, err := indexEntries(ctx, content.Conn, control.Conn, writer, []domain.JournalCatalogEntry{assignment.Entry}, mode, true, makeProvider(&inlineContexts)); err != nil {
				t.Fatal(err)
			}
			var input, output bytes.Buffer
			for _, message := range []any{WorkerBootstrap{ProtocolVersion: WorkerProtocolVersion, WorkerId: 0}, ParentMessage{"committed", WorkerProtocolVersion, 0, 0, 0, 0, false}, ParentMessage{"committed", WorkerProtocolVersion, 0, 1, 0, 1, true}} {
				if err := WriteProtocol(&input, message); err != nil {
					t.Fatal(err)
				}
			}
			if err := RunFetchWorker(ctx, requests[0], &input, &output, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
				return makeProvider(&workerContexts), nil, nil
			}); err != nil {
				t.Fatal(err)
			}
			expected := []domain.IndexFetchContext{{Mode: mode, CommittedAnchor: testCheckpoint("anchor-old"), TraversalCheckpoint: testCheckpoint("cursor-frozen")}, {Mode: mode, CommittedAnchor: testCheckpoint("anchor-old"), TraversalCheckpoint: testCheckpoint("cursor-next")}}
			if !reflect.DeepEqual(inlineContexts, expected) || !reflect.DeepEqual(workerContexts, expected) {
				t.Fatalf("inline=%+v worker=%+v expected=%+v", inlineContexts, workerContexts, expected)
			}
			reader := NewProtocolReader(&output)
			for ordinal, kind := range []string{"batch", "batch", "succeeded"} {
				var message WorkerMessage
				if err := reader.Read(&message); err != nil || message.Type != kind || message.Sequence != uint64(ordinal) {
					t.Fatal(message, err)
				}
			}
		})
	}
}

func TestBootstrapRetainsOldOutboxUntilResumedCatalogCompletes(t *testing.T) {
	ctx := context.Background()
	config := liveFixture(t, "alpha")
	config.ShouldUpdate = false
	filename := filepath.Join(config.ProjectRoot, "data/index/alpha.sqlite")
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		t.Fatal(err)
	}
	connection, err := storage.OpenContent(ctx, filename)
	if err != nil {
		t.Fatal(err)
	}
	_, err = connection.ExecContext(ctx, "INSERT INTO article_change_events(content_revision,article_id,change_kind,journal_id,in_press,created_at) VALUES('old-revision',42,'upsert',1,0,'2026-01-01'); CREATE TRIGGER forbid_bootstrap_events BEFORE INSERT ON article_change_events BEGIN SELECT RAISE(ABORT,'bootstrap emitted event'); END")
	connection.Close()
	if err != nil {
		t.Fatal(err)
	}
	calls := 0
	var originalId int64
	run := liveRunner(t, func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		calls++
		if fetch.Mode != domain.Bootstrap || fetch.CommittedAnchor != nil {
			t.Fatalf("bootstrap context=%+v", fetch)
		}
		if calls == 2 {
			return domain.ProviderBatch{}, errors.New("interrupted bootstrap")
		}
		batch := liveBatch(entry)
		if calls == 1 {
			batch.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("cursor-after-head")}
		} else {
			if fetch.TraversalCheckpoint == nil || *fetch.TraversalCheckpoint != "cursor-after-head" {
				t.Fatalf("checkpoint=%+v", fetch)
			}
			assertLiveContent(t, config, "alpha", 1, 1)
		}
		return batch, nil
	})
	if _, err := runLiveIndex(ctx, config, run, rejectNotify(t)); err == nil {
		t.Fatal("interruption was ignored")
	}
	assertLiveContent(t, config, "alpha", 1, 1)
	connection, err = storage.OpenContent(ctx, filename)
	if err != nil {
		t.Fatal(err)
	}
	var revision string
	if err := connection.QueryRowContext(ctx, "SELECT content_revision FROM article_change_events").Scan(&revision); err != nil || revision != "old-revision" {
		t.Fatal(revision, err)
	}
	if err := connection.QueryRowContext(ctx, "SELECT article_id FROM articles").Scan(&originalId); err != nil {
		t.Fatal(err)
	}
	connection.Close()
	result, err := runLiveIndex(ctx, config, run, rejectNotify(t))
	if err != nil || result.Status != "succeeded" || calls != 3 || result.Csvs[0].ManifestPath != nil {
		t.Fatalf("result=%+v calls=%d err=%v", result, calls, err)
	}
	assertLiveContent(t, config, "alpha", 1, 0)
	connection, err = storage.OpenContent(ctx, filename)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	var resumedId int64
	if err := connection.QueryRowContext(ctx, "SELECT article_id FROM articles").Scan(&resumedId); err != nil || resumedId != originalId {
		t.Fatal(resumedId, originalId, err)
	}
}
