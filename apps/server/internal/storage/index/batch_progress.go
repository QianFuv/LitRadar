package index

import (
	"bytes"
	"context"
	"database/sql"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/batchdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

func validCatalogTransition(current, next CatalogPhase) bool {
	switch current {
	case CatalogPending:
		return next == CatalogIndexing
	case CatalogIndexing:
		return next == CatalogManifestPrepared || next == CatalogCompleted
	case CatalogManifestPrepared:
		return next == CatalogManifestPublished
	case CatalogManifestPublished:
		return next == CatalogNotifying || next == CatalogCompleted
	case CatalogNotifying:
		return next == CatalogCompleted
	}
	return false
}

// TransitionCatalogPhase permits only an idempotent replay or the original forward phase edges.
func TransitionCatalogPhase(ctx context.Context, connection *sql.Conn, id, owner string, ordinal uint64, next CatalogPhase, now int64) error {
	return immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		current, err := readCatalogPhase(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		if current != next && !validCatalogTransition(current, next) {
			return batchState("catalog phase transition is not allowed")
		}
		if _, err := batchQueries.TransitionCatalogPhase(ctx, connection, batchdb.TransitionCatalogPhaseParams{BatchID: sqlite.Text(id), Ordinal: sqlite.Integer(int64(ordinal)), Phase: sqlite.Text(next), UpdatedAt: sqlite.Integer(now)}); err != nil {
			return err
		}
		return touchBatch(ctx, connection, id, now)
	})
}

// StoreManifestIntent commits exact publication bytes before any filesystem publication or outbox acknowledgement.
func StoreManifestIntent(ctx context.Context, connection *sql.Conn, id, owner string, ordinal uint64, intent ManifestIntent, now int64) error {
	if err := validateManifestIntent(intent); err != nil {
		return err
	}
	return immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		phase, err := readCatalogPhase(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		current, err := readManifestIntent(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		if current != nil {
			if !sameManifestIntent(*current, intent) {
				return batchState("catalog already has a different manifest intent")
			}
			if phase == CatalogIndexing {
				return batchState("stored manifest intent has no prepared catalog phase")
			}
		} else {
			if phase != CatalogIndexing {
				return batchState("manifest intent requires an indexing catalog")
			}
			if _, err := batchQueries.StoreManifestIntent(ctx, connection, batchdb.StoreManifestIntentParams{BatchID: sqlite.Text(id), Ordinal: sqlite.Integer(int64(ordinal)), ManifestPayload: dbtypes.Blob(intent.Payload), ManifestSHA256: new(sqlite.Text(intent.Sha256)), ManifestThroughEventID: (*sqlite.Integer)(intent.ThroughEventID), ManifestPath: new(sqlite.Text(intent.Path)), ManifestRunID: new(sqlite.Text(intent.RunID)), ManifestGeneratedAt: new(sqlite.Text(intent.GeneratedAt)), UpdatedAt: sqlite.Integer(now)}); err != nil {
				return err
			}
		}
		return touchBatch(ctx, connection, id, now)
	})
}
func sameManifestIntent(first, second ManifestIntent) bool {
	return bytes.Equal(first.Payload, second.Payload) && first.Sha256 == second.Sha256 && sameOptional(first.ThroughEventID, second.ThroughEventID) && first.Path == second.Path && first.RunID == second.RunID && first.GeneratedAt == second.GeneratedAt
}

// StoreCatalogOutcome records immutable indexing counters while permitting later publication-path enrichment.
func StoreCatalogOutcome(ctx context.Context, connection *sql.Conn, id, owner string, ordinal uint64, outcome BatchCatalogOutcome, now int64) error {
	return immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		if err := saveCatalogOutcome(ctx, connection, id, ordinal, outcome, now); err != nil {
			return err
		}
		return touchBatch(ctx, connection, id, now)
	})
}
func saveCatalogOutcome(ctx context.Context, connection *sql.Conn, id string, ordinal uint64, outcome BatchCatalogOutcome, now int64) error {
	if err := validateCatalogOutcome(outcome); err != nil {
		return err
	}
	phase, err := readCatalogPhase(ctx, connection, id, ordinal)
	if err != nil {
		return err
	}
	if phase == CatalogPending {
		return batchState("catalog outcome cannot be stored before indexing starts")
	}
	current, err := readCatalogOutcome(ctx, connection, id, ordinal)
	if err != nil {
		return err
	}
	outcome, err = retainCatalogOutcome(current, outcome)
	if err != nil {
		return err
	}
	attempts, err := sqliteCount(outcome.SourceAttemptCount)
	if err != nil {
		return err
	}
	journals, err := sqliteCount(outcome.JournalCount)
	if err != nil {
		return err
	}
	changed, err := batchQueries.StoreCatalogOutcome(ctx, connection, batchdb.StoreCatalogOutcomeParams{BatchID: sqlite.Text(id), Ordinal: sqlite.Integer(int64(ordinal)), RunID: new(sqlite.Text(outcome.RunID)), WrittenArticleCount: new(sqlite.Integer(outcome.WrittenArticleCount)), SourceAttemptCount: new(sqlite.Integer(attempts)), OutcomeManifestPath: (*sqlite.Text)(outcome.ManifestPath), UpdatedAt: sqlite.Integer(now), JournalCount: sqlite.Integer(journals)})
	return expectBatchChange(changed, err, "catalog outcome journal count does not match the frozen CSV")
}

// CompleteCatalog requires trusted notification success before leaving the notifying phase.
func CompleteCatalog(ctx context.Context, connection *sql.Conn, id, owner string, ordinal uint64, outcome BatchCatalogOutcome, now int64) error {
	return immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		phase, err := readCatalogPhase(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		if phase != CatalogCompleted && !validCatalogTransition(phase, CatalogCompleted) {
			return batchState("catalog cannot complete from its current phase")
		}
		if phase == CatalogNotifying {
			state, err := readNotifyHandoff(ctx, connection, id, ordinal)
			if err != nil {
				return err
			}
			if state == nil {
				return batchState("notifying catalog has no notification handoff")
			}
			if !state.Status.IsSuccess() {
				return batchState("catalog cannot complete without a trusted notification result")
			}
		}
		if err := saveCatalogOutcome(ctx, connection, id, ordinal, outcome, now); err != nil {
			return err
		}
		if _, err := batchQueries.CompleteCatalog(ctx, connection, batchdb.CompleteCatalogParams{BatchID: sqlite.Text(id), Ordinal: sqlite.Integer(int64(ordinal)), UpdatedAt: sqlite.Integer(now)}); err != nil {
			return err
		}
		return touchBatch(ctx, connection, id, now)
	})
}

// CompleteBatch removes the active marker and its lease only after all catalogs have completed.
func CompleteBatch(ctx context.Context, connection *sql.Conn, id, owner string, now int64) error {
	return immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		var incomplete sqlite.Integer
		if err := func() error {
			queryResult, err := batchQueries.IncompleteCatalogCount(ctx, connection, batchdb.IncompleteCatalogCountParams{BatchID: sqlite.Text(id)})
			if err != nil {
				return err
			}
			incomplete = sqlite.Integer(queryResult)
			return nil
		}(); err != nil {
			return err
		}
		if incomplete != 0 {
			return batchState("batch cannot complete while a catalog is incomplete")
		}
		changed, err := batchQueries.CompleteBatch(ctx, connection, batchdb.CompleteBatchParams{BatchID: sqlite.Text(id), UpdatedAt: sqlite.Integer(now)})
		if err = expectBatchChange(changed, err, "only an active batch can complete"); err != nil {
			return err
		}
		changed, err = batchQueries.ReleaseBatchLease(ctx, connection, batchdb.ReleaseBatchLeaseParams{BatchID: sqlite.Text(id), OwnerID: sqlite.Text(owner)})
		return expectLeaseChange(changed, err, owner)
	})
}

func retainCatalogOutcome(current *BatchCatalogOutcome, outcome BatchCatalogOutcome) (BatchCatalogOutcome, error) {
	if current != nil {
		if current.RunID != outcome.RunID || current.JournalCount != outcome.JournalCount || current.WrittenArticleCount != outcome.WrittenArticleCount || current.SourceAttemptCount != outcome.SourceAttemptCount {
			return outcome, batchState("catalog outcome immutable counters changed during recovery")
		}
		if current.ManifestPath != nil {
			if outcome.ManifestPath != nil && *current.ManifestPath != *outcome.ManifestPath {
				return outcome, batchState("catalog manifest path changed during recovery")
			}
			outcome.ManifestPath = current.ManifestPath
		}
	}
	return outcome, nil
}
