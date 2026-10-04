package index

import (
	"context"
	"database/sql"
	"reflect"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

const runCondition = "catalog_name=?1 AND provider_name=?2 AND catalog_id=?3 AND batch_id=?4 AND run_id=?5 AND sync_mode=?6 AND base_anchor IS ?7"
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
			changed, err := connection.ExecContext(ctx, `UPDATE provider_run_checkpoints SET run_id=?5,updated_at=?9 WHERE catalog_name=?1 AND provider_name=?2 AND catalog_id=?3 AND batch_id=?4 AND run_id=?6 AND sync_mode=?7 AND base_anchor IS ?8`, run.Scope.CatalogName, run.Scope.ProviderName, run.Scope.CatalogId, run.BatchId, run.RunId, existing.RunId, run.Mode, desired, updatedAt)
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

func runArguments(run SyncRun) []any {
	return []any{run.Scope.CatalogName, run.Scope.ProviderName, run.Scope.CatalogId, run.BatchId, run.RunId, run.Mode, run.BaseAnchor}
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
	result, err := connection.ExecContext(ctx, "UPDATE provider_run_checkpoints SET traversal_checkpoint=?8,updated_at=?9 WHERE "+runCondition, append(runArguments(run), checkpoint, updatedAt)...)
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
	var exists sqlite.Integer
	if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM provider_run_checkpoints WHERE "+runCondition+")", runArguments(run)...).Scan(&exists); err != nil {
		return err
	}
	if exists == 0 {
		return &ControlError{Kind: "run_ownership_lost", RunId: run.RunId}
	}
	return nil
}

func completeRun(ctx context.Context, connection *sql.Conn, run SyncRun, nextAnchor *string, completedAt string) error {
	if err := verifyJournalRun(ctx, connection, run); err != nil {
		return err
	}
	if _, err := connection.ExecContext(ctx, `INSERT INTO provider_sync_anchors(catalog_name,provider_name,catalog_id,committed_anchor,completed_at,completed_batch_id) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(catalog_name,provider_name,catalog_id) DO UPDATE SET committed_anchor=excluded.committed_anchor,completed_at=excluded.completed_at,completed_batch_id=excluded.completed_batch_id`, run.Scope.CatalogName, run.Scope.ProviderName, run.Scope.CatalogId, nextAnchor, completedAt, run.BatchId); err != nil {
		return err
	}
	result, err := connection.ExecContext(ctx, "DELETE FROM provider_run_checkpoints WHERE "+runCondition, runArguments(run)...)
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
		if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2 AND run_id=?3 AND expires_at>unixepoch())", run.Scope.CatalogName, run.Scope.ProviderName, run.RunId).Scan(&exists); err != nil {
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
