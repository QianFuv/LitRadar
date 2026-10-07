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

// batchCatalogRecovery owns the latest persisted outcome and fall-through recovery phase.
type batchCatalogRecovery struct {
	ctx         context.Context
	connection  *sql.Conn
	config      LiveConfig
	batch       storage.IndexBatch
	stored      storage.IndexBatchCatalog
	input       storage.CatalogInput
	phase       storage.CatalogPhase
	persisted   *storage.BatchCatalogOutcome
	intent      *storage.ManifestIntent
	concurrency LiveConcurrency
}

// runBatchCatalogs validates the full ledger count before honoring the selected stop boundary.
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
		recovery := batchCatalogRecovery{ctx: ctx, connection: connection, config: config, batch: batch, stored: stored, input: input, phase: stored.Phase, persisted: stored.Outcome, intent: stored.ManifestIntent, concurrency: concurrency}
		outcome, err := recovery.resume(run, notify)
		if err != nil {
			return nil, err
		}
		outcomes = append(outcomes, outcome)
	}
	return outcomes, nil
}

// transition changes the in-memory phase only after its durable transition succeeds.
func (recovery *batchCatalogRecovery) transition(next storage.CatalogPhase) error {
	if err := storage.TransitionCatalogPhase(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, next, time.Now().Unix()); err != nil {
		return err
	}
	recovery.phase = next
	return nil
}

// complete records the latest outcome, including any persisted manifest path enrichment.
func (recovery *batchCatalogRecovery) complete(handoff *storage.NotifyHandoffState) (LiveCatalogOutcome, error) {
	if err := storage.CompleteCatalog(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, *recovery.persisted, time.Now().Unix()); err != nil {
		return LiveCatalogOutcome{}, err
	}
	return recovery.outcome(handoff), nil
}

// outcome reports persisted counters and the observed or resumed executor concurrency.
func (recovery *batchCatalogRecovery) outcome(handoff *storage.NotifyHandoffState) LiveCatalogOutcome {
	return catalogOutcome(recovery.config, recovery.input, *recovery.persisted, handoff, recovery.concurrency)
}

// resume advances successive durable phases in the same invocation.
func (recovery *batchCatalogRecovery) resume(run catalogRunner, notify notifyRunner) (LiveCatalogOutcome, error) {
	if recovery.phase == storage.CatalogCompleted {
		if recovery.persisted == nil {
			return LiveCatalogOutcome{}, &storage.BatchError{Kind: "state", Reason: "completed catalog has no persisted outcome"}
		}
		return recovery.outcome(recovery.stored.NotifyHandoff), nil
	}
	if err := recovery.index(run); err != nil {
		return LiveCatalogOutcome{}, err
	}
	if !recovery.config.ShouldUpdate {
		return recovery.completeWithoutManifest()
	}
	if err := recovery.recoverManifest(); err != nil {
		return LiveCatalogOutcome{}, err
	}
	return recovery.finishPublished(notify)
}

// index persists the indexing outcome before entering any manifest phase.
func (recovery *batchCatalogRecovery) index(run catalogRunner) error {
	if recovery.phase == storage.CatalogPending {
		if err := recovery.transition(storage.CatalogIndexing); err != nil {
			return err
		}
	}
	if recovery.phase == storage.CatalogIndexing && recovery.persisted == nil {
		observed, err := run(recovery.ctx, recovery.config, recovery.input, recovery.batch.BatchId)
		if err != nil {
			return err
		}
		recovery.concurrency = observed.Concurrency
		value := storage.BatchCatalogOutcome{RunId: observed.RunId, JournalCount: observed.JournalCount, WrittenArticleCount: observed.WrittenArticleCount, SourceAttemptCount: observed.SourceAttemptCount}
		if err := storage.StoreCatalogOutcome(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, value, time.Now().Unix()); err != nil {
			return err
		}
		recovery.persisted = &value
	}
	if recovery.persisted == nil {
		return &storage.BatchError{Kind: "state", Reason: "catalog finalization has no persisted indexing outcome"}
	}
	return nil
}

// completeWithoutManifest rejects update recovery state in a non-update batch.
func (recovery *batchCatalogRecovery) completeWithoutManifest() (LiveCatalogOutcome, error) {
	if recovery.phase != storage.CatalogIndexing || recovery.intent != nil {
		return LiveCatalogOutcome{}, &storage.BatchError{Kind: "state", Reason: "non-update catalog has manifest recovery state"}
	}
	return recovery.complete(nil)
}

// recoverManifest freezes its payload before validating and publishing its durable intent.
func (recovery *batchCatalogRecovery) recoverManifest() error {
	if recovery.phase == storage.CatalogIndexing {
		prepared, err := prepareCatalogManifest(recovery.ctx, recovery.config, recovery.input, *recovery.persisted)
		if err != nil {
			return err
		}
		if err := storage.StoreManifestIntent(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, prepared, time.Now().Unix()); err != nil {
			return err
		}
		recovery.intent = &prepared
		recovery.phase = storage.CatalogManifestPrepared
	}
	if recovery.intent == nil {
		return &storage.BatchError{Kind: "state", Reason: "update catalog finalization has no manifest intent"}
	}
	if err := validateManifestRecovery(recovery.input, *recovery.persisted, *recovery.intent); err != nil {
		return err
	}
	return recovery.publishManifest()
}

// publishManifest persists the publication path before writing files or acknowledging events.
func (recovery *batchCatalogRecovery) publishManifest() error {
	shouldPublish := recovery.persisted.ManifestPath != nil
	if recovery.phase == storage.CatalogManifestPrepared {
		var err error
		shouldPublish, err = shouldPublishManifest(recovery.config, recovery.input, *recovery.persisted, *recovery.intent)
		if err != nil {
			return err
		}
	}
	if shouldPublish && recovery.persisted.ManifestPath == nil {
		copied := *recovery.persisted
		copied.ManifestPath = &recovery.intent.Path
		recovery.persisted = &copied
		if err := storage.StoreCatalogOutcome(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, *recovery.persisted, time.Now().Unix()); err != nil {
			return err
		}
	}
	if recovery.phase == storage.CatalogManifestPrepared {
		if shouldPublish {
			if err := publishCatalogManifest(recovery.ctx, recovery.config, recovery.input, *recovery.intent); err != nil {
				return err
			}
		}
		if err := recovery.transition(storage.CatalogManifestPublished); err != nil {
			return err
		}
	}
	return nil
}

// finishPublished enters notification only for a published manifest requiring handoff.
func (recovery *batchCatalogRecovery) finishPublished(notify notifyRunner) (LiveCatalogOutcome, error) {
	if recovery.phase == storage.CatalogManifestPublished {
		if recovery.config.ShouldNotify && recovery.persisted.ManifestPath != nil {
			if err := recovery.transition(storage.CatalogNotifying); err != nil {
				return LiveCatalogOutcome{}, err
			}
		} else {
			return recovery.complete(nil)
		}
	}
	if recovery.phase == storage.CatalogNotifying {
		handoff, err := recovery.notify(notify)
		if err != nil {
			return LiveCatalogOutcome{}, err
		}
		return recovery.complete(&handoff)
	}
	return LiveCatalogOutcome{}, &storage.BatchError{Kind: "state", Reason: "catalog recovery stopped in an unsupported phase"}
}

// notify preserves persisted successful attempts and blocks ambiguous handoffs until acknowledged.
func (recovery *batchCatalogRecovery) notify(run notifyRunner) (storage.NotifyHandoffState, error) {
	if recovery.persisted.ManifestPath == nil {
		return storage.NotifyHandoffState{}, &storage.BatchError{Kind: "state", Reason: "notifying catalog has no published manifest outcome"}
	}
	prepared, err := storage.PrepareNotifyAttempt(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, storage.NewNotifyAttemptId(), recovery.config.ShouldAcknowledgeUnknownNotify, time.Now().Unix())
	if err != nil {
		return storage.NotifyHandoffState{}, err
	}
	handoff := prepared.State
	if prepared.Decision == "blocked_unknown" {
		return storage.NotifyHandoffState{}, fmt.Errorf("notification handoff is ambiguous; review it and rerun with --acknowledge-unknown-notify")
	}
	if prepared.Decision == "run" {
		return recovery.recordNotification(run, handoff)
	}
	return handoff, nil
}

// recordNotification persists failures before reporting them, with recorder errors taking precedence.
func (recovery *batchCatalogRecovery) recordNotification(run notifyRunner, handoff storage.NotifyHandoffState) (storage.NotifyHandoffState, error) {
	observed, err := run(recovery.ctx, NotifyProcessConfig{recovery.config.ApplicationExecutable, recovery.config.SecretKeyFile, recovery.config.ProjectRoot, recovery.config.IsNotifyDryRun}, catalogDatabaseName(recovery.input), filepath.Join(recovery.config.ProjectRoot, recovery.intent.Path), handoff.AttemptId)
	if err != nil {
		if _, recordError := storage.RecordNotifyAttemptResult(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, handoff.AttemptId, storage.NotifyFailed, nil, time.Now().Unix()); recordError != nil {
			return storage.NotifyHandoffState{}, recordError
		}
		return storage.NotifyHandoffState{}, err
	}
	handoff, err = storage.RecordNotifyAttemptResult(recovery.ctx, recovery.connection, recovery.batch.BatchId, recovery.batch.OwnerId, recovery.stored.Ordinal, handoff.AttemptId, observed.Status, observed.ExitCode, time.Now().Unix())
	if err != nil {
		return storage.NotifyHandoffState{}, err
	}
	if !handoff.Status.IsSuccess() {
		return storage.NotifyHandoffState{}, fmt.Errorf("notification handoff ended with status %s", handoff.Status)
	}
	return handoff, nil
}
