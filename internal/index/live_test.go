package index

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func liveFixture(t *testing.T, names ...string) LiveConfig {
	t.Helper()
	config := LiveConfig{ProjectRoot: t.TempDir(), IssueBatchSize: 8, TimeoutSeconds: 20, ShouldResume: true, ShouldUpdate: true, IndexProviderRoutes: map[string]string{}}
	meta := filepath.Join(config.ProjectRoot, "data", "meta")
	if err := os.MkdirAll(meta, 0700); err != nil {
		t.Fatal(err)
	}
	for _, name := range names {
		row := make([]string, len(catalogColumns))
		row[0], row[2] = "journal-"+name, "Journal "+name
		body := strings.Join(catalogColumns, ",") + "\n" + strings.Join(row, ",") + "\n"
		if err := os.WriteFile(filepath.Join(meta, name+".csv"), []byte(body), 0600); err != nil {
			t.Fatal(err)
		}
		config.IndexProviderRoutes[name] = "cnki"
	}
	return config
}

func liveBatch(entry domain.JournalCatalogEntry) domain.ProviderBatch {
	return domain.ProviderBatch{CatalogId: entry.CatalogId, Journal: domain.JournalDraft{CatalogId: entry.CatalogId}, Articles: []domain.ArticleDraft{{CatalogId: entry.CatalogId, Title: "Durable article", Doi: testCheckpoint("10.1234/" + entry.CatalogId), Authors: []domain.ArticleAuthorDraft{}, RetractionDois: []string{}}}, Progress: domain.ProviderProgress{State: domain.Complete, NextAnchor: testCheckpoint("committed-anchor")}}
}

func liveRunner(t *testing.T, fetch workerTestProvider) catalogRunner {
	t.Helper()
	return func(ctx context.Context, config LiveConfig, input storage.CatalogInput, batch string) (LiveCatalogOutcome, error) {
		return runLiveCatalogWithFactory(ctx, config, input, batch, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) { return fetch, nil, nil })
	}
}

func rejectNotify(t *testing.T) notifyRunner {
	return func(context.Context, NotifyProcessConfig, string, string, string) (NotifyObservation, error) {
		t.Fatal("unexpected notification")
		return NotifyObservation{}, nil
	}
}

func assertLiveContent(t *testing.T, config LiveConfig, name string, articles, events int) {
	t.Helper()
	connection, err := storage.OpenContent(context.Background(), filepath.Join(config.ProjectRoot, "data", "index", name+".sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	for table, expected := range map[string]int{"articles": articles, "article_change_events": events} {
		var count int
		if err := connection.Conn.QueryRowContext(context.Background(), "SELECT count(*) FROM "+table).Scan(&count); err != nil || count != expected {
			t.Fatalf("%s count=%d expected=%d err=%v", table, count, expected, err)
		}
	}
}

func TestLiveBoundaryResumeAndNoChangePreservePublication(t *testing.T) {
	ctx := context.Background()
	config := liveFixture(t, "alpha", "beta")
	config.StopAfter = testCheckpoint("alpha.csv")
	calls := map[string]int{}
	run := liveRunner(t, func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		calls[entry.CatalogId]++
		return liveBatch(entry), nil
	})
	first, err := runLiveIndex(ctx, config, run, rejectNotify(t))
	if err != nil || first.Status != "paused" || len(first.Csvs) != 1 || first.Csvs[0].WrittenArticleCount != 1 || first.Csvs[0].SourceAttemptCount != 1 || first.Csvs[0].Concurrency.InlineExecutorCount != 1 {
		t.Fatalf("first=%+v err=%v", first, err)
	}
	manifest := *first.Csvs[0].ManifestPath
	original, err := os.ReadFile(manifest)
	if err != nil {
		t.Fatal(err)
	}
	assertLiveContent(t, config, "alpha", 1, 0)
	config.StopAfter = nil
	second, err := runLiveIndex(ctx, config, run, rejectNotify(t))
	if err != nil || second.Status != "succeeded" || len(second.Csvs) != 2 || second.Csvs[0].RunId != first.Csvs[0].RunId || second.Csvs[0].Concurrency.ExecutorCount != 0 || calls["journal-alpha"] != 1 || calls["journal-beta"] != 1 {
		t.Fatalf("second=%+v calls=%v err=%v", second, calls, err)
	}
	third, err := runLiveIndex(ctx, config, run, rejectNotify(t))
	if err != nil || third.Status != "succeeded" || third.Csvs[0].WrittenArticleCount != 0 || third.Csvs[0].ManifestPath != nil {
		t.Fatalf("unchanged=%+v err=%v", third, err)
	}
	current, err := os.ReadFile(manifest)
	if err != nil || !bytes.Equal(original, current) {
		t.Fatalf("no-change run replaced prior manifest: %v", err)
	}
	assertLiveContent(t, config, "alpha", 1, 0)
}

func TestLiveResumePublishesFrozenIntentAfterFilesystemFailure(t *testing.T) {
	ctx := context.Background()
	config := liveFixture(t, "alpha")
	blocked := filepath.Join(config.ProjectRoot, "data", "push_state")
	if err := os.WriteFile(blocked, []byte("publication blocker"), 0600); err != nil {
		t.Fatal(err)
	}
	calls := 0
	run := liveRunner(t, func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		calls++
		return liveBatch(entry), nil
	})
	if _, err := runLiveIndex(ctx, config, run, rejectNotify(t)); err == nil {
		t.Fatal("publication failure was ignored")
	}
	assertLiveContent(t, config, "alpha", 1, 1)
	if err := os.Remove(blocked); err != nil {
		t.Fatal(err)
	}
	recovered, err := runLiveIndex(ctx, config, run, rejectNotify(t))
	if err != nil || recovered.Status != "succeeded" || calls != 1 || recovered.Csvs[0].Concurrency.ExecutorCount != 0 || recovered.Csvs[0].ManifestPath == nil {
		t.Fatalf("recovered=%+v calls=%d err=%v", recovered, calls, err)
	}
	assertLiveContent(t, config, "alpha", 1, 0)
	body, err := os.ReadFile(*recovered.Csvs[0].ManifestPath)
	if err != nil {
		t.Fatal(err)
	}
	var manifest map[string]any
	if err := json.Unmarshal(body, &manifest); err != nil || manifest["run_id"] != recovered.Csvs[0].RunId {
		t.Fatalf("manifest=%v err=%v", manifest, err)
	}
}

func TestLiveUnknownNotificationRequiresAcknowledgmentAndNewAttempt(t *testing.T) {
	ctx := context.Background()
	config := liveFixture(t, "alpha")
	config.ShouldNotify = true
	fetches, notifications := 0, []string{}
	run := liveRunner(t, func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		fetches++
		return liveBatch(entry), nil
	})
	notify := func(ctx context.Context, options NotifyProcessConfig, database, manifest, attempt string) (NotifyObservation, error) {
		if database != "alpha.sqlite" {
			t.Fatalf("database=%s", database)
		}
		if _, err := os.Stat(manifest); err != nil {
			t.Fatal(err)
		}
		assertLiveContent(t, config, "alpha", 1, 0)
		notifications = append(notifications, attempt)
		if len(notifications) == 1 {
			return NotifyObservation{Status: storage.NotifyUnknown}, nil
		}
		zero := int32(0)
		return NotifyObservation{Status: storage.NotifyCompleted, ExitCode: &zero}, nil
	}
	if _, err := runLiveIndex(ctx, config, run, notify); err == nil || !strings.Contains(err.Error(), "unknown") {
		t.Fatalf("first error=%v", err)
	}
	if _, err := runLiveIndex(ctx, config, run, notify); err == nil || !strings.Contains(err.Error(), "ambiguous") || len(notifications) != 1 {
		t.Fatalf("unacknowledged retry=%v calls=%v", err, notifications)
	}
	config.ShouldAcknowledgeUnknownNotify = true
	result, err := runLiveIndex(ctx, config, run, notify)
	if err != nil || result.Status != "succeeded" || len(notifications) != 2 || notifications[0] == notifications[1] || fetches != 1 {
		t.Fatalf("result=%+v attempts=%v fetches=%d err=%v", result, notifications, fetches, err)
	}
}

func TestLiveFailedPageResumesCommittedCheckpoint(t *testing.T) {
	ctx := context.Background()
	config := liveFixture(t, "alpha")
	calls := 0
	run := liveRunner(t, func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
		calls++
		if calls == 2 {
			return domain.ProviderBatch{}, errors.New("synthetic fetch failure")
		}
		batch := liveBatch(entry)
		if calls == 1 {
			batch.Progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: testCheckpoint("durable-page")}
		} else if fetch.TraversalCheckpoint == nil || *fetch.TraversalCheckpoint != "durable-page" {
			t.Fatalf("lost checkpoint: %+v", fetch)
		}
		return batch, nil
	})
	if _, err := runLiveIndex(ctx, config, run, rejectNotify(t)); err == nil || !strings.Contains(err.Error(), "synthetic fetch failure") {
		t.Fatalf("first=%v", err)
	}
	assertLiveContent(t, config, "alpha", 1, 1)
	result, err := runLiveIndex(ctx, config, run, rejectNotify(t))
	if err != nil || calls != 3 || result.Csvs[0].WrittenArticleCount != 0 || result.Csvs[0].SourceAttemptCount != 1 {
		t.Fatalf("result=%+v calls=%d err=%v", result, calls, err)
	}
	assertLiveContent(t, config, "alpha", 1, 0)
}

func TestCatalogSelectionExcludesDotfileAndSortsBoundaries(t *testing.T) {
	config := liveFixture(t, "beta", "alpha")
	meta := filepath.Join(config.ProjectRoot, "data", "meta")
	for _, filename := range []string{".csv", "upper.CSV", "plain"} {
		if err := os.WriteFile(filepath.Join(meta, filename), nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	paths, err := catalogPaths(meta, nil)
	if err != nil || len(paths) != 2 || filepath.Base(paths[0]) != "alpha.csv" || filepath.Base(paths[1]) != "beta.csv" {
		t.Fatalf("paths=%v err=%v", paths, err)
	}
	if _, err := catalogPaths(meta, testCheckpoint(".csv")); err == nil {
		t.Fatal("dotfile was treated as an extension")
	}
}

func TestHeartbeatStopsAndRetainsFailure(t *testing.T) {
	expected := errors.New("lease lost")
	heartbeat := startLeaseHeartbeat(func() error { return expected }, "panic", time.Millisecond)
	select {
	case <-heartbeat.done:
	case <-time.After(time.Second):
		t.Fatal("heartbeat did not stop on lease loss")
	}
	if heartbeat.stopAndCheck() != expected || heartbeat.stopAndCheck() != expected {
		t.Fatal("heartbeat lost its failure")
	}
}

func TestLiveRejectsIncompleteCatalogLedgerBeforeAnyExecution(t *testing.T) {
	for _, mutation := range []string{"DELETE FROM index_batch_catalogs WHERE ordinal=1", "INSERT INTO index_batch_catalogs(batch_id,ordinal,file_name,catalog_name,csv_sha256,provider_name,journal_count,phase,updated_at) SELECT batch_id,2,'extra.csv','extra',csv_sha256,provider_name,journal_count,'pending',updated_at FROM index_batch_catalogs WHERE ordinal=0"} {
		t.Run(mutation[:6], func(t *testing.T) {
			ctx := context.Background()
			config := liveFixture(t, "alpha", "beta")
			inputs := []storage.CatalogInput{}
			for _, name := range []string{"alpha", "beta"} {
				input, err := FreezeCatalog(filepath.Join(config.ProjectRoot, "data", "meta", name+".csv"), "cnki")
				if err != nil {
					t.Fatal(err)
				}
				inputs = append(inputs, input)
			}
			request, err := storage.NewBatchRequest(inputs, "all", domain.Incremental, 8, false, false)
			if err != nil {
				t.Fatal(err)
			}
			connection, err := storage.OpenBatch(ctx, filepath.Join(config.ProjectRoot, "data", "index-control", storage.BatchDatabaseFilename))
			if err != nil {
				t.Fatal(err)
			}
			defer connection.Close()
			admission, err := storage.AdmitBatch(ctx, connection.Conn, request, true, "test-owner", time.Now().Unix())
			if err != nil {
				t.Fatal(err)
			}
			if _, err := connection.Conn.ExecContext(ctx, mutation); err != nil {
				t.Fatal(err)
			}
			_, err = runBatchCatalogs(ctx, connection.Conn, config, request, admission.Batch, 0, func(context.Context, LiveConfig, storage.CatalogInput, string) (LiveCatalogOutcome, error) {
				t.Fatal("ran with a corrupt catalog ledger")
				return LiveCatalogOutcome{}, nil
			}, rejectNotify(t))
			if err == nil || !strings.Contains(err.Error(), "active batch catalog count changed after admission") {
				t.Fatalf("error=%v", err)
			}
		})
	}
}

func TestLiveProviderCleanupRunsDuringPanic(t *testing.T) {
	config := liveFixture(t, "alpha")
	input, err := FreezeCatalog(filepath.Join(config.ProjectRoot, "data", "meta", "alpha.csv"), "cnki")
	if err != nil {
		t.Fatal(err)
	}
	closed := false
	func() {
		defer func() {
			if recover() != "synthetic-provider-panic" {
				t.Error("provider panic was lost")
			}
		}()
		_, _ = runLiveCatalogWithFactory(context.Background(), config, input, "batch", func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
			return workerTestProvider(func(context.Context, domain.JournalCatalogEntry, domain.IndexFetchContext) (domain.ProviderBatch, error) {
				panic("synthetic-provider-panic")
			}), func() { closed = true }, nil
		})
	}()
	if !closed {
		t.Fatal("provider resources survived panic")
	}
}
