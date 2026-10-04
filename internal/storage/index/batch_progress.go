package index

import (
	"bytes"
	"context"
	"database/sql"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
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
		if _, err := connection.ExecContext(ctx, `UPDATE index_batch_catalogs SET phase=?3,updated_at=?4 WHERE batch_id=?1 AND ordinal=?2`, id, int64(ordinal), next, now); err != nil {
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
			if _, err := connection.ExecContext(ctx, `UPDATE index_batch_catalogs SET phase='manifest_prepared',manifest_payload=?3,manifest_sha256=?4,manifest_through_event_id=?5,manifest_path=?6,manifest_run_id=?7,manifest_generated_at=?8,updated_at=?9 WHERE batch_id=?1 AND ordinal=?2`, id, int64(ordinal), intent.Payload, intent.Sha256, intent.ThroughEventId, intent.Path, intent.RunId, intent.GeneratedAt, now); err != nil {
				return err
			}
		}
		return touchBatch(ctx, connection, id, now)
	})
}
func sameManifestIntent(first, second ManifestIntent) bool {
	return bytes.Equal(first.Payload, second.Payload) && first.Sha256 == second.Sha256 && sameOptional(first.ThroughEventId, second.ThroughEventId) && first.Path == second.Path && first.RunId == second.RunId && first.GeneratedAt == second.GeneratedAt
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
	if current != nil {
		if current.RunId != outcome.RunId || current.JournalCount != outcome.JournalCount || current.WrittenArticleCount != outcome.WrittenArticleCount || current.SourceAttemptCount != outcome.SourceAttemptCount {
			return batchState("catalog outcome immutable counters changed during recovery")
		}
		if current.ManifestPath != nil {
			if outcome.ManifestPath != nil && *current.ManifestPath != *outcome.ManifestPath {
				return batchState("catalog manifest path changed during recovery")
			}
			outcome.ManifestPath = current.ManifestPath
		}
	}
	attempts, err := sqliteCount(outcome.SourceAttemptCount)
	if err != nil {
		return err
	}
	journals, err := sqliteCount(outcome.JournalCount)
	if err != nil {
		return err
	}
	changed, err := connection.ExecContext(ctx, `UPDATE index_batch_catalogs SET run_id=?3,written_article_count=?4,source_attempt_count=?5,outcome_manifest_path=?6,updated_at=?7 WHERE batch_id=?1 AND ordinal=?2 AND journal_count=?8`, id, int64(ordinal), outcome.RunId, outcome.WrittenArticleCount, attempts, outcome.ManifestPath, now, journals)
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
		if _, err := connection.ExecContext(ctx, `UPDATE index_batch_catalogs SET phase='completed',completed_at=COALESCE(completed_at,?3),updated_at=?3 WHERE batch_id=?1 AND ordinal=?2`, id, int64(ordinal), now); err != nil {
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
		if err := connection.QueryRowContext(ctx, `SELECT COUNT(*) FROM index_batch_catalogs WHERE batch_id=?1 AND phase!='completed'`, id).Scan(&incomplete); err != nil {
			return err
		}
		if incomplete != 0 {
			return batchState("batch cannot complete while a catalog is incomplete")
		}
		changed, err := connection.ExecContext(ctx, `UPDATE index_batches SET status='completed',updated_at=?2,completed_at=?2 WHERE batch_id=?1 AND status='active'`, id, now)
		if err = expectBatchChange(changed, err, "only an active batch can complete"); err != nil {
			return err
		}
		changed, err = connection.ExecContext(ctx, `DELETE FROM index_batch_lease WHERE lease_key=1 AND batch_id=?1 AND owner_id=?2`, id, owner)
		return expectLeaseChange(changed, err, owner)
	})
}
