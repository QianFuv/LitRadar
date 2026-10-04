package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/index"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
	"github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func prepareRecoveredIndex(t *testing.T, ctx context.Context) string {
	t.Helper()
	root := t.TempDir()
	for _, directory := range []string{"meta", "index-control", "index-work/scholarly"} {
		if err := os.MkdirAll(filepath.Join(root, "data", directory), 0700); err != nil {
			t.Fatal(err)
		}
	}
	filename := filepath.Join(root, "data", "auth.sqlite")
	if _, err := auth.Migrate(ctx, filename); err != nil {
		t.Fatal(err)
	}
	database, err := sqlite.Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec(`INSERT INTO runtime_settings(key,value,updated_at) VALUES('index_provider_routes','{"fixture":"scholarly"}',1)`); err != nil {
		t.Fatal(err)
	}
	database.Close()
	var requests atomic.Int64
	proxy := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) { requests.Add(1); writer.WriteHeader(503) }))
	t.Cleanup(func() {
		proxy.Close()
		if requests.Load() != 0 {
			t.Errorf("offline recovery attempted %d upstream requests", requests.Load())
		}
	})
	repository, err := authstorage.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	proxyUrl, policy := proxy.URL, `{"scholarly":true}`
	apiKey, mailto := "synthetic-offline-key", "fixture@example.invalid"
	_, err = settings.New(repository, codec).Update(ctx, nil, map[string]*string{"provider_proxy_url": &proxyUrl, "provider_proxy_policy": &policy, "openalex_api_key_pool": &apiKey, "semantic_scholar_api_key_pool": &apiKey, "crossref_mailto_pool": &mailto}, nil, nil)
	repository.Close()
	codec.Close()
	if err != nil {
		t.Fatal(err)
	}
	header := "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating"
	csv := header + "\n"
	for ordinal := range 2 {
		row := make([]string, 16)
		row[0], row[2], row[3] = fmt.Sprintf("fixture-%d", ordinal), fmt.Sprintf("Offline Journal %d", ordinal), []string{"1234-5679", "2049-3630"}[ordinal]
		row[5] = row[3]
		csv += strings.Join(row, ",") + "\n"
	}
	catalogPath := filepath.Join(root, "data", "meta", "fixture.csv")
	if err := os.WriteFile(catalogPath, []byte(csv), 0600); err != nil {
		t.Fatal(err)
	}
	catalog, err := index.FreezeCatalog(catalogPath, "scholarly")
	if err != nil {
		t.Fatal(err)
	}
	request, err := storage.NewBatchRequest([]storage.CatalogInput{catalog}, "explicit_file", domain.Incremental, 8, true, true)
	if err != nil {
		t.Fatal(err)
	}
	batch, err := storage.OpenBatch(ctx, filepath.Join(root, "data", "index-control", storage.BatchDatabaseFilename))
	if err != nil {
		t.Fatal(err)
	}
	defer batch.Close()
	admission, err := storage.AdmitBatch(ctx, batch.Conn, request, true, "fixture-owner", time.Now().Unix())
	if err != nil {
		t.Fatal(err)
	}
	if err := storage.TransitionCatalogPhase(ctx, batch.Conn, admission.Batch.BatchId, "fixture-owner", 0, storage.CatalogIndexing, time.Now().Unix()); err != nil {
		t.Fatal(err)
	}
	control, err := storage.OpenControl(ctx, filepath.Join(root, "data", "index-control", "fixture.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer control.Close()
	if err := storage.AcquireLease(ctx, control.Conn, "fixture", "scholarly", "fixture-run", time.Now().Unix()); err != nil {
		t.Fatal(err)
	}
	for ordinal, entry := range catalog.Entries {
		var work map[string]any
		decoder := json.NewDecoder(strings.NewReader(`{"title":["Offline recovered article"],"type":"journal-article","created":{"date-time":"2026-01-01T00:00:00Z","timestamp":1767225600000},"issued":{"date-parts":[[2026,1,1]]},"volume":"1","issue":"1"}`))
		decoder.UseNumber()
		if err := decoder.Decode(&work); err != nil {
			t.Fatal(err)
		}
		work["title"] = []any{fmt.Sprintf("Recovered article %d", ordinal)}
		provider := scholarly.NewIndexProvider(scholarly.NewFixtureTransport(scholarly.FixtureData{CrossrefWorks: []any{work}}), false, filepath.Join(root, "data", "index-work", "scholarly"))
		var checkpoint *string
		for range 2 {
			page, err := provider.Fetch(ctx, entry, domain.IndexFetchContext{Mode: domain.Incremental, TraversalCheckpoint: checkpoint})
			if err != nil || page.Progress.Checkpoint == nil {
				t.Fatal("could not prepare collected upstream replay", page.Progress, err)
			}
			checkpoint = page.Progress.Checkpoint
		}
		if !strings.Contains(*checkpoint, `"ready"`) {
			t.Fatal("fixture still requires an upstream request", *checkpoint)
		}
		run := storage.SyncRun{Scope: storage.SyncScope{CatalogName: "fixture", ProviderName: "scholarly", CatalogId: entry.CatalogId}, BatchId: admission.Batch.BatchId, RunId: "fixture-run", Mode: domain.Incremental}
		if _, err := storage.PrepareJournalSync(ctx, control.Conn, run, true, "fixture"); err != nil {
			t.Fatal(err)
		}
		if err := storage.AdvanceRunCheckpoint(ctx, control.Conn, run, *checkpoint, "fixture"); err != nil {
			t.Fatal(err)
		}
	}
	if err := storage.ReleaseLease(ctx, control.Conn, "fixture", "scholarly", "fixture-run"); err != nil {
		t.Fatal(err)
	}
	if err := storage.ReleaseBatchLease(ctx, batch.Conn, admission.Batch.BatchId, "fixture-owner"); err != nil {
		t.Fatal(err)
	}
	return root
}

func verifyRecoveredIndex(t *testing.T, root, stdout, stderr string) {
	t.Helper()
	var result index.LiveRunOutcome
	if err := json.Unmarshal([]byte(stdout), &result); err != nil {
		t.Fatal("index output lost its JSON boundary", stdout, err)
	}
	if result.Status != "succeeded" || len(result.Csvs) != 1 {
		t.Fatal(result)
	}
	catalog := result.Csvs[0]
	if catalog.Concurrency.ChildProcessCount != 2 || catalog.WrittenArticleCount != 2 || catalog.SourceAttemptCount != 4 || catalog.NotifyExitCode == nil || *catalog.NotifyExitCode != 0 || catalog.ManifestPath == nil {
		t.Fatalf("same executable worker/notify chain incomplete: %+v", catalog)
	}
	processes := map[string]map[int]bool{"index": {}, "notify": {}}
	for _, line := range strings.Split(stderr, "\n") {
		var event struct {
			Event string
			Span  struct {
				Command   string
				ProcessId int `json:"process_id"`
			}
		}
		if json.Unmarshal([]byte(line), &event) == nil && event.Event == "process.started" && processes[event.Span.Command] != nil {
			processes[event.Span.Command][event.Span.ProcessId] = true
		}
	}
	if len(processes["index"]) != 3 || len(processes["notify"]) != 1 {
		t.Fatal("missing child command correlation", stderr)
	}
	connection, err := storage.OpenContent(context.Background(), filepath.Join(root, "data", "index", "fixture.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	var count int
	if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM articles").Scan(&count); err != nil || count != 2 {
		t.Fatal("acknowledged content not durable", count, err)
	}
	manifest, err := os.ReadFile(*catalog.ManifestPath)
	if err != nil {
		t.Fatal(err)
	}
	var published struct {
		Database string  `json:"db_name"`
		Run      string  `json:"run_id"`
		Articles []int64 `json:"notifiable_article_ids"`
	}
	if json.Unmarshal(manifest, &published) != nil || published.Database != "fixture.sqlite" || published.Run != catalog.RunId || len(published.Articles) != 2 {
		t.Fatal("incorrect published change manifest", string(manifest))
	}
	history, err := os.ReadFile(filepath.Join(root, "data", "push_state", "history", "fixture", fmt.Sprintf("%x.changes.json", sha256.Sum256(manifest))))
	if err != nil || !bytes.Equal(history, manifest) {
		t.Fatal("history differs from published manifest", err)
	}
	if err := connection.QueryRowContext(context.Background(), "SELECT count(*) FROM article_change_events").Scan(&count); err != nil || count != 0 {
		t.Fatal("published changes were not acknowledged", count, err)
	}
	for ordinal := range 2 {
		var article int64
		if err := connection.QueryRowContext(context.Background(), "SELECT article_id FROM articles WHERE title=?", "Recovered article "+strconv.Itoa(ordinal)).Scan(&article); err != nil || !slices.Contains(published.Articles, article) {
			t.Fatal("child article lost from published manifest", ordinal, article, err)
		}
	}
	batch, err := storage.OpenBatch(context.Background(), filepath.Join(root, "data", "index-control", storage.BatchDatabaseFilename))
	if err != nil {
		t.Fatal(err)
	}
	defer batch.Close()
	var batchId, status string
	if err := batch.QueryRowContext(context.Background(), "SELECT batch_id,status FROM index_batches").Scan(&batchId, &status); err != nil || status != "completed" {
		t.Fatal("batch did not complete", status, err)
	}
	catalogs, err := storage.ReadBatchCatalogs(context.Background(), batch.Conn, batchId)
	if err != nil || len(catalogs) != 1 || catalogs[0].Phase != storage.CatalogCompleted || catalogs[0].NotifyHandoff == nil || !catalogs[0].NotifyHandoff.Status.IsSuccess() {
		t.Fatal("notify handoff was not durable", catalogs, err)
	}
	if err := batch.QueryRowContext(context.Background(), "SELECT count(*) FROM index_batch_lease").Scan(&count); err != nil || count != 0 {
		t.Fatal("batch lease was not released", count, err)
	}
	control, err := storage.OpenControl(context.Background(), filepath.Join(root, "data", "index-control", "fixture.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer control.Close()
	for _, table := range []string{"provider_leases", "provider_run_checkpoints"} {
		if err := control.QueryRowContext(context.Background(), "SELECT count(*) FROM "+table).Scan(&count); err != nil || count != 0 {
			t.Fatal("completed recovery state remains", table, count, err)
		}
	}
	if err := control.QueryRowContext(context.Background(), "SELECT count(*) FROM provider_sync_anchors WHERE completed_batch_id=?", batchId).Scan(&count); err != nil || count != 2 {
		t.Fatal("journal completion was not acknowledged", count, err)
	}
}
