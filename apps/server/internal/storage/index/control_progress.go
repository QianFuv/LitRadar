package index

import (
	"context"
	"database/sql"
	"reflect"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/controldb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

const insertRunSql = `INSERT INTO provider_run_checkpoints(catalog_name,provider_name,catalog_id,batch_id,run_id,sync_mode,base_anchor,traversal_checkpoint,started_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,NULL,?8,?8)`

// PrepareJournalSync freezes the anchor and either resumes its exact batch or replaces traversal state.
func PrepareJournalSync(ctx context.Context, connection *sql.Conn, run SyncRun, shouldResume bool, updatedAt string) (JournalSyncPreparation, error) {
	var result JournalSyncPreparation
	if err := validateBatchId(run.BatchId); err != nil {
		return result, err
	}
	if err := validateRunMetadata(run.RunId, updatedAt); err != nil {
		return result, err
	}
	err := immediate(ctx, connection, func() error {
		anchor, err := ReadSyncAnchor(ctx, connection, run.Scope)
		if err != nil {
			return err
		}
		if shouldResume && anchor != nil && anchor.CompletedBatchId != nil && *anchor.CompletedBatchId == run.BatchId {
			result.ShouldSkip = true
			return nil
		}
		var desired *string
		if run.Mode != domain.Bootstrap && anchor != nil {
			desired = copyValue(anchor.CommittedAnchor)
		}
		existing, err := ReadRunCheckpoint(ctx, connection, run.Scope)
		if err != nil {
			return err
		}
		if shouldResume && existing != nil {
			if existing.BatchId == nil || *existing.BatchId != run.BatchId {
				return &ControlError{Kind: "batch_mismatch"}
			}
			if existing.Mode != run.Mode {
				return &ControlError{Kind: "mode_mismatch", Stored: existing.Mode, Requested: run.Mode}
			}
			if !reflect.DeepEqual(existing.BaseAnchor, desired) {
				return invalidSync("run base anchor does not match the committed anchor")
			}
			changed, err := controlQueries.ResumeRun(ctx, connection, controldb.ResumeRunParams{CatalogName: sqlite.Text(run.Scope.CatalogName), ProviderName: sqlite.Text(run.Scope.ProviderName), CatalogID: sqlite.Text(run.Scope.CatalogId), BatchID: new(sqlite.Text(run.BatchId)), RunID: sqlite.Text(run.RunId), RunID_2: sqlite.Text(existing.RunId), SyncMode: sqlite.Text(run.Mode), BaseAnchor: (*sqlite.Text)(desired), UpdatedAt: sqlite.Text(updatedAt)})
			if err := requireRunChange(changed, err, run.RunId); err != nil {
				return err
			}
			existing.RunId = run.RunId
			existing.UpdatedAt = updatedAt
			result.Checkpoint = existing
			return nil
		}
		statement := insertRunSql
		if !shouldResume {
			statement += ` ON CONFLICT(catalog_name,provider_name,catalog_id) DO UPDATE SET batch_id=excluded.batch_id,run_id=excluded.run_id,sync_mode=excluded.sync_mode,base_anchor=excluded.base_anchor,traversal_checkpoint=NULL,started_at=excluded.started_at,updated_at=excluded.updated_at`
		}
		if _, err := connection.ExecContext(ctx, statement, run.Scope.CatalogName, run.Scope.ProviderName, run.Scope.CatalogId, run.BatchId, run.RunId, run.Mode, desired, updatedAt); err != nil {
			return err
		}
		result.Checkpoint = &ProviderRunCheckpoint{BatchId: ptr(run.BatchId), RunId: run.RunId, Mode: run.Mode, BaseAnchor: desired, StartedAt: updatedAt, UpdatedAt: updatedAt}
		return nil
	})
	if err != nil {
		return JournalSyncPreparation{}, err
	}
	return result, nil
}

func requireRunChange(result sql.Result, err error, run string) error {
	if err != nil {
		return err
	}
	count, err := result.RowsAffected()
	if err != nil {
		return err
	}
	if count != 1 {
		return &ControlError{Kind: "run_ownership_lost", RunId: run}
	}
	return nil
}

func validateProgress(run SyncRun, value *string, field, updatedAt string) error {
	if err := validateBatchId(run.BatchId); err != nil {
		return err
	}
	if err := validateRunMetadata(run.RunId, updatedAt); err != nil {
		return err
	}
	if err := validateOpaque(run.BaseAnchor, "base anchor"); err != nil {
		return err
	}
	return validateOpaque(value, field)
}

func advanceRun(ctx context.Context, connection *sql.Conn, run SyncRun, checkpoint, updatedAt string) error {
	result, err := controlQueries.AdvanceRun(ctx, connection, controldb.AdvanceRunParams{CatalogName: sqlite.Text(run.Scope.CatalogName), ProviderName: sqlite.Text(run.Scope.ProviderName), CatalogID: sqlite.Text(run.Scope.CatalogId), BatchID: new(sqlite.Text(run.BatchId)), RunID: sqlite.Text(run.RunId), SyncMode: sqlite.Text(run.Mode), BaseAnchor: (*sqlite.Text)(run.BaseAnchor), TraversalCheckpoint: new(sqlite.Text(checkpoint)), UpdatedAt: sqlite.Text(updatedAt)})
	return requireRunChange(result, err, run.RunId)
}

// AdvanceRunCheckpoint advances an exact journal owner without imposing the composition API's lease check.
func AdvanceRunCheckpoint(ctx context.Context, connection *sql.Conn, run SyncRun, checkpoint, updatedAt string) error {
	if err := validateProgress(run, &checkpoint, "traversal checkpoint", updatedAt); err != nil {
		return err
	}
	return immediate(ctx, connection, func() error { return advanceRun(ctx, connection, run, checkpoint, updatedAt) })
}

func verifyJournalRun(ctx context.Context, connection *sql.Conn, run SyncRun) error {
	exists, err := controlQueries.OwnsJournalRun(ctx, connection, controldb.OwnsJournalRunParams{CatalogName: sqlite.Text(run.Scope.CatalogName), ProviderName: sqlite.Text(run.Scope.ProviderName), CatalogID: sqlite.Text(run.Scope.CatalogId), BatchID: new(sqlite.Text(run.BatchId)), RunID: sqlite.Text(run.RunId), SyncMode: sqlite.Text(run.Mode), BaseAnchor: (*sqlite.Text)(run.BaseAnchor)})
	if err != nil {
		return err
	}
	if !exists {
		return &ControlError{Kind: "run_ownership_lost", RunId: run.RunId}
	}
	return nil
}

func completeRun(ctx context.Context, connection *sql.Conn, run SyncRun, nextAnchor *string, completedAt string) error {
	if err := verifyJournalRun(ctx, connection, run); err != nil {
		return err
	}
	if _, err := controlQueries.CompleteRun(ctx, connection, controldb.CompleteRunParams{CatalogName: sqlite.Text(run.Scope.CatalogName), ProviderName: sqlite.Text(run.Scope.ProviderName), CatalogID: sqlite.Text(run.Scope.CatalogId), CommittedAnchor: (*sqlite.Text)(nextAnchor), CompletedAt: sqlite.Text(completedAt), CompletedBatchID: new(sqlite.Text(run.BatchId))}); err != nil {
		return err
	}
	result, err := controlQueries.DeleteRun(ctx, connection, controldb.DeleteRunParams{CatalogName: sqlite.Text(run.Scope.CatalogName), ProviderName: sqlite.Text(run.Scope.ProviderName), CatalogID: sqlite.Text(run.Scope.CatalogId), BatchID: new(sqlite.Text(run.BatchId)), RunID: sqlite.Text(run.RunId), SyncMode: sqlite.Text(run.Mode), BaseAnchor: (*sqlite.Text)(run.BaseAnchor)})
	return requireRunChange(result, err, run.RunId)
}

// CompleteSyncRun atomically replaces exact journal traversal ownership with a successful anchor.
func CompleteSyncRun(ctx context.Context, connection *sql.Conn, run SyncRun, nextAnchor *string, completedAt string) error {
	if err := validateProgress(run, nextAnchor, "committed anchor", completedAt); err != nil {
		return err
	}
	return immediate(ctx, connection, func() error { return completeRun(ctx, connection, run, nextAnchor, completedAt) })
}

// ContentCheckpointError identifies which independently durable operation failed.
type ContentCheckpointError struct {
	Phase string
	Cause error
}

func (err *ContentCheckpointError) Error() string {
	if err.Phase == "content" {
		return "content commit failed: " + err.Cause.Error()
	}
	return "sync progress commit failed: " + err.Cause.Error()
}
func (err *ContentCheckpointError) Unwrap() error { return err.Cause }

// CommitContentThenProgress fences ownership while committing content before disposable progress.
// A later control failure retains committed content for idempotent replay; callers may acknowledge only success.
func CommitContentThenProgress[T any](ctx context.Context, connection *sql.Conn, run SyncRun, progress domain.ProviderProgress, updatedAt string, writeContent func() (T, error)) (T, error) {
	var outcome T
	field := "committed anchor"
	value := progress.NextAnchor
	if progress.State == domain.Continue {
		field = "traversal checkpoint"
		value = progress.Checkpoint
		if value == nil {
			return outcome, &ContentCheckpointError{"control", invalidSync(field)}
		}
	} else if progress.State != domain.Complete {
		return outcome, &ContentCheckpointError{"control", invalidSync("provider progress state is invalid")}
	}
	if err := validateProgress(run, value, field, updatedAt); err != nil {
		return outcome, &ContentCheckpointError{"control", err}
	}
	phase := "control"
	err := immediate(ctx, connection, func() error {
		var exists sqlite.Integer
		if err := func() error {
			queryResult, err := controlQueries.OwnsLiveLease(ctx, connection, controldb.OwnsLiveLeaseParams{CatalogName: sqlite.Text(run.Scope.CatalogName), ProviderName: sqlite.Text(run.Scope.ProviderName), RunID: sqlite.Text(run.RunId)})
			if err != nil {
				return err
			}
			exists = sqlite.Integer(dbtypes.Boolean(queryResult))
			return nil
		}(); err != nil {
			return err
		}
		if exists == 0 {
			return &ControlError{Kind: "ownership_lost", RunId: run.RunId}
		}
		if err := verifyJournalRun(ctx, connection, run); err != nil {
			return err
		}
		var err error
		outcome, err = writeContent()
		if err != nil {
			phase = "content"
			return err
		}
		if progress.State == domain.Continue {
			return advanceRun(ctx, connection, run, *value, updatedAt)
		}
		return completeRun(ctx, connection, run, value, updatedAt)
	})
	if err != nil {
		var zero T
		return zero, &ContentCheckpointError{phase, err}
	}
	return outcome, nil
}
