package index

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/platform/process"
	"github.com/QianFuv/LitRadar/internal/provider"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

type crashReady struct {
	Payload []byte `json:"payload"`
	Attempt string `json:"attempt"`
}

func crashConfiguration(root string) LiveConfig {
	return LiveConfig{ProjectRoot: root, IssueBatchSize: 8, TimeoutSeconds: 20, ShouldResume: true, ShouldUpdate: true, ShouldNotify: true, IndexProviderRoutes: map[string]string{"alpha": "cnki"}}
}

func runCrashFixture(root, point string) error {
	ctx := context.Background()
	config := crashConfiguration(root)
	input, err := FreezeCatalog(filepath.Join(root, "data", "meta", "alpha.csv"), "cnki")
	if err != nil {
		return err
	}
	request, err := storage.NewBatchRequest([]storage.CatalogInput{input}, "all", domain.Incremental, 8, true, false)
	if err != nil {
		return err
	}
	connection, err := storage.OpenBatch(ctx, filepath.Join(root, "data", "index-control", storage.BatchDatabaseFilename))
	if err != nil {
		return err
	}
	defer connection.Close()
	admission, err := storage.AdmitBatch(ctx, connection.Conn, request, true, "crash-owner", time.Now().Unix())
	if err != nil {
		return err
	}
	batch := admission.Batch
	ready := crashReady{}
	boundary := func(name string) {
		if name != point {
			return
		}
		body, err := json.Marshal(ready)
		if err == nil {
			err = os.WriteFile(filepath.Join(root, "ready.tmp"), body, 0600)
		}
		if err == nil {
			err = os.Rename(filepath.Join(root, "ready.tmp"), filepath.Join(root, "ready.json"))
		}
		if err != nil {
			panic(err)
		}
		for {
			time.Sleep(time.Hour)
		}
	}
	boundary("admitted")
	transition := func(phase storage.CatalogPhase) error {
		return storage.TransitionCatalogPhase(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, phase, time.Now().Unix())
	}
	if err := transition(storage.CatalogIndexing); err != nil {
		return err
	}
	boundary("indexing")
	if point == "content" {
		if err := os.MkdirAll(filepath.Dir(catalogContentPath(config, input)), 0700); err != nil {
			return err
		}
		control, err := storage.OpenControl(ctx, catalogControlPath(config, "alpha"))
		if err != nil {
			return err
		}
		defer control.Close()
		content, err := storage.OpenContent(ctx, catalogContentPath(config, input))
		if err != nil {
			return err
		}
		defer content.Close()
		run := storage.SyncRun{Scope: storage.SyncScope{CatalogName: "alpha", ProviderName: "cnki", CatalogId: input.Entries[0].CatalogId}, BatchId: batch.BatchId, RunId: "crash-run", Mode: domain.Incremental}
		if err := storage.AcquireLease(ctx, control.Conn, "alpha", "cnki", run.RunId, time.Now().Unix()); err != nil {
			return err
		}
		if _, err := storage.PrepareJournalSync(ctx, control.Conn, run, true, "epoch"); err != nil {
			return err
		}
		page := liveBatch(input.Entries[0])
		_, err = storage.CommitContentThenProgress(ctx, control.Conn, run, page.Progress, "epoch", func() (storage.ContentWriteOutcome, error) {
			outcome, err := storage.WriteContentBatchWithEvents(ctx, content.Conn, input.Entries[0], page, "crash-revision", "epoch", true)
			if err == nil {
				boundary("content")
			}
			return outcome, err
		})
		return err
	}
	observed, err := runLiveCatalogWithFactory(ctx, config, input, batch.BatchId, func(WorkerRequest, WorkerBootstrap) (provider.IndexContent, func(), error) {
		return workerTestProvider(func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
			return liveBatch(entry), nil
		}), nil, nil
	})
	if err != nil {
		return err
	}
	boundary("progress")
	outcome := storage.BatchCatalogOutcome{RunId: observed.RunId, JournalCount: observed.JournalCount, WrittenArticleCount: observed.WrittenArticleCount, SourceAttemptCount: observed.SourceAttemptCount}
	if err := storage.StoreCatalogOutcome(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, outcome, time.Now().Unix()); err != nil {
		return err
	}
	boundary("outcome")
	intent, err := prepareCatalogManifest(ctx, config, input, outcome)
	if err != nil {
		return err
	}
	if err := storage.StoreManifestIntent(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, intent, time.Now().Unix()); err != nil {
		return err
	}
	ready.Payload = intent.Payload
	boundary("intent")
	outcome.ManifestPath = &intent.Path
	if err := storage.StoreCatalogOutcome(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, outcome, time.Now().Unix()); err != nil {
		return err
	}
	boundary("manifest-path")
	if err := storage.PublishContentChangeHistory(filepath.Join(catalogHistoryDirectory(config, input), intent.Sha256+".changes.json"), intent.Payload); err != nil {
		return err
	}
	boundary("history")
	if err := storage.PublishContentChangeManifest(filepath.Join(root, intent.Path), intent.Payload); err != nil {
		return err
	}
	boundary("current")
	content, err := storage.OpenContent(ctx, catalogContentPath(config, input))
	if err != nil {
		return err
	}
	_, err = storage.AcknowledgeContentChangeEvents(ctx, content.Conn, *intent.ThroughEventId)
	content.Close()
	if err != nil {
		return err
	}
	boundary("ack")
	if err := transition(storage.CatalogManifestPublished); err != nil {
		return err
	}
	boundary("published")
	if err := transition(storage.CatalogNotifying); err != nil {
		return err
	}
	boundary("notifying")
	attempt, err := storage.PrepareNotifyAttempt(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, "durable-notification-attempt", false, time.Now().Unix())
	if err != nil {
		return err
	}
	ready.Attempt = attempt.State.AttemptId
	boundary("notify-running")
	zero := int32(0)
	if _, err := storage.RecordNotifyAttemptResult(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, ready.Attempt, storage.NotifyCompleted, &zero, time.Now().Unix()); err != nil {
		return err
	}
	boundary("notify-completed")
	if err := storage.CompleteCatalog(ctx, connection.Conn, batch.BatchId, batch.OwnerId, 0, outcome, time.Now().Unix()); err != nil {
		return err
	}
	boundary("catalog-completed")
	return fmt.Errorf("unknown crash point %s", point)
}

func TestActualKillRestartAtEveryIndexDurableBoundary(t *testing.T) {
	executable, err := os.Executable()
	if err != nil {
		t.Fatal(err)
	}
	for _, point := range []string{"admitted", "indexing", "content", "progress", "outcome", "intent", "manifest-path", "history", "current", "ack", "published", "notifying", "notify-running", "notify-completed", "catalog-completed"} {
		t.Run(point, func(t *testing.T) {
			seed := liveFixture(t, "alpha")
			config := crashConfiguration(seed.ProjectRoot)
			ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
			defer cancel()
			child, err := process.Start(ctx, process.Config{Path: executable, Args: []string{"--index-crash-fixture"}, Environment: append(os.Environ(), "LITRADAR_INDEX_CRASH_ROOT="+config.ProjectRoot, "LITRADAR_INDEX_CRASH_POINT="+point), OutputLimit: 4096})
			if err != nil {
				t.Fatal(err)
			}
			defer child.Close()
			var ready crashReady
			ticker := time.NewTicker(10 * time.Millisecond)
			defer ticker.Stop()
			for {
				body, err := os.ReadFile(filepath.Join(config.ProjectRoot, "ready.json"))
				if err == nil {
					if err := json.Unmarshal(body, &ready); err != nil {
						t.Fatal(err)
					}
					break
				}
				select {
				case <-ticker.C:
				case <-ctx.Done():
					diagnostics, _ := child.Diagnostics()
					t.Fatalf("child did not reach %s: %s", point, diagnostics)
				}
			}
			if err := child.Close(); err != nil {
				t.Fatal(err)
			}
			if err := child.Wait(ctx); err == nil {
				t.Fatal("child was not killed")
			}
			batch, err := storage.OpenBatch(ctx, filepath.Join(config.ProjectRoot, "data", "index-control", storage.BatchDatabaseFilename))
			if err != nil {
				t.Fatal(err)
			}
			_, err = batch.Conn.ExecContext(ctx, "UPDATE index_batch_lease SET expires_at=0")
			batch.Close()
			if err != nil {
				t.Fatal(err)
			}
			control, err := storage.OpenControl(ctx, catalogControlPath(config, "alpha"))
			if err != nil {
				t.Fatal(err)
			}
			_, err = control.Conn.ExecContext(ctx, "UPDATE provider_leases SET expires_at=0")
			control.Close()
			if err != nil {
				t.Fatal(err)
			}
			fetches, notifications := 0, 0
			run := liveRunner(t, func(ctx context.Context, entry domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
				fetches++
				return liveBatch(entry), nil
			})
			notify := func(ctx context.Context, options NotifyProcessConfig, database, manifest, attempt string) (NotifyObservation, error) {
				notifications++
				if ready.Attempt != "" && ready.Attempt != attempt {
					t.Fatal("running notification lost stable attempt identity")
				}
				zero := int32(0)
				return NotifyObservation{Status: storage.NotifyCompleted, ExitCode: &zero}, nil
			}
			result, err := runLiveIndex(ctx, config, run, notify)
			if err != nil || result.Status != "succeeded" || len(result.Csvs) != 1 {
				t.Fatalf("resume=%+v err=%v", result, err)
			}
			expectedFetches := 0
			if point == "admitted" || point == "indexing" || point == "content" {
				expectedFetches = 1
			}
			expectedNotifications := 1
			if point == "notify-completed" || point == "catalog-completed" {
				expectedNotifications = 0
			}
			if fetches != expectedFetches || notifications != expectedNotifications {
				t.Fatalf("fetches=%d expected=%d notifications=%d expected=%d", fetches, expectedFetches, notifications, expectedNotifications)
			}
			assertLiveContent(t, config, "alpha", 1, 0)
			if len(ready.Payload) > 0 {
				body, err := os.ReadFile(*result.Csvs[0].ManifestPath)
				if err != nil || !bytes.Equal(body, ready.Payload) {
					t.Fatalf("recovery changed frozen publication: %v", err)
				}
			}
		})
	}
}
