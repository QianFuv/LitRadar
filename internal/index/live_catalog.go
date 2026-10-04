package index

import (
	"context"
	"database/sql"
	"fmt"
	"path/filepath"
	"time"

	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

type catalogRunner func(context.Context, LiveConfig, storage.CatalogInput, string) (LiveCatalogOutcome, error)
type notifyRunner func(context.Context, NotifyProcessConfig, string, string, string) (NotifyObservation, error)

func runBatchCatalogs(ctx context.Context, connection *sql.Conn, config LiveConfig, request storage.BatchRequest, batch storage.IndexBatch, stopAfter int, run catalogRunner, notify notifyRunner) ([]LiveCatalogOutcome, error) {
	catalogs, err := storage.ReadBatchCatalogs(ctx, connection, batch.BatchId)
	if err != nil {
		return nil, err
	}
	if len(catalogs) != len(request.Catalogs) {
		return nil, &storage.BatchError{Kind: "state", Reason: "active batch catalog count changed after admission"}
	}
	outcomes := []LiveCatalogOutcome{}
	for position, stored := range catalogs {
		if position > stopAfter {
			break
		}
		input := request.Catalogs[position]
		if stored.Filename != input.Filename || stored.CatalogName != input.CatalogName {
			return nil, &storage.BatchError{Kind: "state", Reason: "active batch catalog order changed after admission"}
		}
		concurrency, err := catalogConcurrency(config, input.ProviderName)
		if err != nil {
			return nil, err
		}
		phase, persisted, intent := stored.Phase, stored.Outcome, stored.ManifestIntent
		transition := func(next storage.CatalogPhase) error {
			if err := storage.TransitionCatalogPhase(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, next, time.Now().Unix()); err != nil {
				return err
			}
			phase = next
			return nil
		}
		complete := func(handoff *storage.NotifyHandoffState) error {
			if err := storage.CompleteCatalog(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, *persisted, time.Now().Unix()); err != nil {
				return err
			}
			outcomes = append(outcomes, catalogOutcome(config, input, *persisted, handoff, concurrency))
			return nil
		}
		if phase == storage.CatalogCompleted {
			if persisted == nil {
				return nil, &storage.BatchError{Kind: "state", Reason: "completed catalog has no persisted outcome"}
			}
			outcomes = append(outcomes, catalogOutcome(config, input, *persisted, stored.NotifyHandoff, concurrency))
			continue
		}
		if phase == storage.CatalogPending {
			if err := transition(storage.CatalogIndexing); err != nil {
				return nil, err
			}
		}
		if phase == storage.CatalogIndexing && persisted == nil {
			observed, err := run(ctx, config, input, batch.BatchId)
			if err != nil {
				return nil, err
			}
			concurrency = observed.Concurrency
			value := storage.BatchCatalogOutcome{RunId: observed.RunId, JournalCount: observed.JournalCount, WrittenArticleCount: observed.WrittenArticleCount, SourceAttemptCount: observed.SourceAttemptCount}
			if err := storage.StoreCatalogOutcome(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, value, time.Now().Unix()); err != nil {
				return nil, err
			}
			persisted = &value
		}
		if persisted == nil {
			return nil, &storage.BatchError{Kind: "state", Reason: "catalog finalization has no persisted indexing outcome"}
		}
		if !config.ShouldUpdate {
			if phase != storage.CatalogIndexing || intent != nil {
				return nil, &storage.BatchError{Kind: "state", Reason: "non-update catalog has manifest recovery state"}
			}
			if err := complete(nil); err != nil {
				return nil, err
			}
			continue
		}
		if phase == storage.CatalogIndexing {
			prepared, err := prepareCatalogManifest(ctx, config, input, *persisted)
			if err != nil {
				return nil, err
			}
			if err := storage.StoreManifestIntent(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, prepared, time.Now().Unix()); err != nil {
				return nil, err
			}
			intent = &prepared
			phase = storage.CatalogManifestPrepared
		}
		if intent == nil {
			return nil, &storage.BatchError{Kind: "state", Reason: "update catalog finalization has no manifest intent"}
		}
		if err := validateManifestRecovery(input, *persisted, *intent); err != nil {
			return nil, err
		}
		shouldPublish := persisted.ManifestPath != nil
		if phase == storage.CatalogManifestPrepared {
			shouldPublish, err = shouldPublishManifest(config, input, *persisted, *intent)
			if err != nil {
				return nil, err
			}
		}
		if shouldPublish && persisted.ManifestPath == nil {
			copied := *persisted
			copied.ManifestPath = &intent.Path
			persisted = &copied
			if err := storage.StoreCatalogOutcome(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, *persisted, time.Now().Unix()); err != nil {
				return nil, err
			}
		}
		if phase == storage.CatalogManifestPrepared {
			if shouldPublish {
				if err := publishCatalogManifest(ctx, config, input, *intent); err != nil {
					return nil, err
				}
			}
			if err := transition(storage.CatalogManifestPublished); err != nil {
				return nil, err
			}
		}
		if phase == storage.CatalogManifestPublished {
			if config.ShouldNotify && persisted.ManifestPath != nil {
				if err := transition(storage.CatalogNotifying); err != nil {
					return nil, err
				}
			} else {
				if err := complete(nil); err != nil {
					return nil, err
				}
				continue
			}
		}
		if phase == storage.CatalogNotifying {
			if persisted.ManifestPath == nil {
				return nil, &storage.BatchError{Kind: "state", Reason: "notifying catalog has no published manifest outcome"}
			}
			prepared, err := storage.PrepareNotifyAttempt(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, storage.NewNotifyAttemptId(), config.ShouldAcknowledgeUnknownNotify, time.Now().Unix())
			if err != nil {
				return nil, err
			}
			handoff := prepared.State
			if prepared.Decision == "blocked_unknown" {
				return nil, fmt.Errorf("notification handoff is ambiguous; review it and rerun with --acknowledge-unknown-notify")
			}
			if prepared.Decision == "run" {
				observed, err := notify(ctx, NotifyProcessConfig{config.ApplicationExecutable, config.SecretKeyFile, config.ProjectRoot, config.IsNotifyDryRun}, catalogDatabaseName(input), filepath.Join(config.ProjectRoot, intent.Path), handoff.AttemptId)
				if err != nil {
					if _, recordError := storage.RecordNotifyAttemptResult(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, handoff.AttemptId, storage.NotifyFailed, nil, time.Now().Unix()); recordError != nil {
						return nil, recordError
					}
					return nil, err
				}
				handoff, err = storage.RecordNotifyAttemptResult(ctx, connection, batch.BatchId, batch.OwnerId, stored.Ordinal, handoff.AttemptId, observed.Status, observed.ExitCode, time.Now().Unix())
				if err != nil {
					return nil, err
				}
				if !handoff.Status.IsSuccess() {
					return nil, fmt.Errorf("notification handoff ended with status %s", handoff.Status)
				}
			}
			if err := complete(&handoff); err != nil {
				return nil, err
			}
			continue
		}
		return nil, &storage.BatchError{Kind: "state", Reason: "catalog recovery stopped in an unsupported phase"}
	}
	return outcomes, nil
}
