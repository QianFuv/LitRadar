package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/index"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	authstore "github.com/QianFuv/LitRadar/internal/storage/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

func prepareIndex(root string, shouldNotify bool) error {
	marker, err := os.ReadFile(filepath.Join(root, ".litradar-e2e-root"))
	if err != nil || string(marker) != "litradar-full-stack-e2e-v1\n" {
		return fmt.Errorf("synthetic root marker required")
	}
	ctx := context.Background()
	for _, directory := range []string{"meta", "index-control", "index-work/scholarly"} {
		if err := os.MkdirAll(filepath.Join(root, "data", directory), 0700); err != nil {
			return err
		}
	}
	repository, err := authstore.Open(filepath.Join(root, "data/auth.sqlite"))
	if err != nil {
		return err
	}
	defer repository.Close()
	codec, err := secrets.Load(filepath.Join(filepath.Dir(root), "seed", "secret.key"))
	if err != nil {
		return err
	}
	defer codec.Close()
	apiKey, mailto, routes := "synthetic-offline-key", "fixture@example.invalid", `{"fixture":"scholarly"}`
	if _, err := settings.New(repository, codec).Update(ctx, nil, map[string]*string{"index_provider_routes": &routes, "openalex_api_key_pool": &apiKey, "semantic_scholar_api_key_pool": &apiKey, "crossref_mailto_pool": &mailto}, nil, nil); err != nil {
		return err
	}
	header := "catalog_id,catalog_aliases,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating"
	csv := header + "\n"
	for ordinal := range 2 {
		row := make([]string, 16)
		row[0], row[2], row[3] = fmt.Sprintf("fixture-%d", ordinal), fmt.Sprintf("Offline Journal %d", ordinal), []string{"1234-5679", "2049-3630"}[ordinal]
		row[5] = row[3]
		csv += strings.Join(row, ",") + "\n"
	}
	catalogPath := filepath.Join(root, "data/meta/fixture.csv")
	if err := os.WriteFile(catalogPath, []byte(csv), 0600); err != nil {
		return err
	}
	catalog, err := index.FreezeCatalog(catalogPath, "scholarly")
	if err != nil {
		return err
	}
	request, err := storage.NewBatchRequest([]storage.CatalogInput{catalog}, "explicit_file", domain.Incremental, 8, shouldNotify, shouldNotify)
	if err != nil {
		return err
	}
	batch, err := storage.OpenBatch(ctx, filepath.Join(root, "data/index-control", storage.BatchDatabaseFilename))
	if err != nil {
		return err
	}
	defer batch.Close()
	admission, err := storage.AdmitBatch(ctx, batch.Conn, request, true, "fixture-owner", time.Now().Unix())
	if err != nil {
		return err
	}
	if err := storage.TransitionCatalogPhase(ctx, batch.Conn, admission.Batch.BatchId, "fixture-owner", 0, storage.CatalogIndexing, time.Now().Unix()); err != nil {
		return err
	}
	control, err := storage.OpenControl(ctx, filepath.Join(root, "data/index-control/fixture.sqlite"))
	if err != nil {
		return err
	}
	defer control.Close()
	if err := storage.AcquireLease(ctx, control.Conn, "fixture", "scholarly", "fixture-run", time.Now().Unix()); err != nil {
		return err
	}
	checkpoints := []string{}
	for ordinal, entry := range catalog.Entries {
		run := storage.SyncRun{Scope: storage.SyncScope{CatalogName: "fixture", ProviderName: "scholarly", CatalogId: entry.CatalogId}, BatchId: admission.Batch.BatchId, RunId: "fixture-run", Mode: domain.Incremental}
		prepared, err := storage.PrepareJournalSync(ctx, control.Conn, run, true, "fixture")
		if err != nil {
			return err
		}
		if prepared.ShouldSkip || prepared.Checkpoint == nil {
			return fmt.Errorf("synthetic preparation requires a new batch")
		}
		run.BaseAnchor = prepared.Checkpoint.BaseAnchor
		var work map[string]any
		decoder := json.NewDecoder(strings.NewReader(`{"title":["Offline recovered article"],"type":"journal-article","created":{"date-time":"2026-01-01T00:00:00Z","timestamp":1767225600000},"issued":{"date-parts":[[2026,1,1]]},"volume":"1","issue":"1"}`))
		decoder.UseNumber()
		if err := decoder.Decode(&work); err != nil {
			return err
		}
		work["title"] = []any{fmt.Sprintf("Recovered article %d", ordinal)}
		created := time.Now().UTC().Add(-time.Minute)
		work["created"] = map[string]any{"date-time": created.Format(time.RFC3339), "timestamp": created.UnixMilli()}
		provider := scholarly.NewIndexProvider(scholarly.NewFixtureTransport(scholarly.FixtureData{CrossrefWorks: []any{work}}), false, filepath.Join(root, "data/index-work/scholarly"))
		var checkpoint *string
		for range 8 {
			page, err := provider.Fetch(ctx, entry, domain.IndexFetchContext{Mode: domain.Incremental, CommittedAnchor: run.BaseAnchor, TraversalCheckpoint: checkpoint})
			if err != nil {
				return err
			}
			if page.Progress.Checkpoint == nil {
				return fmt.Errorf("offline fixture checkpoint missing")
			}
			checkpoint = page.Progress.Checkpoint
			if strings.Contains(*checkpoint, `"ready"`) {
				break
			}
		}
		if !strings.Contains(*checkpoint, `"ready"`) {
			return fmt.Errorf("fixture still needs external collection: %s", *checkpoint)
		}
		if err := storage.AdvanceRunCheckpoint(ctx, control.Conn, run, *checkpoint, "fixture"); err != nil {
			return err
		}
		checkpoints = append(checkpoints, *checkpoint)
	}
	if err := storage.ReleaseLease(ctx, control.Conn, "fixture", "scholarly", "fixture-run"); err != nil {
		return err
	}
	if err := storage.ReleaseBatchLease(ctx, batch.Conn, admission.Batch.BatchId, "fixture-owner"); err != nil {
		return err
	}
	oracleRequest := map[string]any{
		"catalogs":  []any{map[string]any{"path": catalog.Path, "filename": catalog.Filename, "name": catalog.CatalogName, "digest": catalog.CsvSha256, "provider": catalog.ProviderName, "entries": catalog.Entries}},
		"selection": "explicit_file", "mode": "incremental", "size": 8, "notify": shouldNotify, "dry": shouldNotify,
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"origin": "Go synthetic preparation; independently frozen workset format proofs remain separate", "checkpoints": checkpoints, "batchId": admission.Batch.BatchId, "oracleRequest": oracleRequest})
}
