package delivery

import (
	"context"
	"database/sql"
)

// RunFinalization returns the three rows committed by one run/checkpoint/lease transaction.
type RunFinalization struct {
	Run        *RunRecord
	Checkpoint *CheckpointRecord
	Lease      *LeaseRecord
}

// FinalizeRunWithCheckpoint advances checkpoint, terminal run and released lease together or rolls them all back.
func (repository *Repository) FinalizeRunWithCheckpoint(ctx context.Context, runId int64, owner string, runRevision int64, status RunStatus, result, errorCode *string, workflow Workflow, dbName string, checkpointRevision *int64, update CheckpointUpdate, leaseRevision int64) (RunFinalization, error) {
	if err := validateRunFinalization(runId, owner, runRevision, status, result, errorCode, dbName, checkpointRevision, update, leaseRevision); err != nil {
		return RunFinalization{}, err
	}
	var completed RunFinalization
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := writeCheckpoint(ctx, connection, workflow, dbName, checkpointRevision, update); err != nil {
			return err
		}
		if err := executeCas(ctx, connection, `UPDATE delivery_runs SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND workflow=? AND db_name=? AND owner_id=? AND revision=? AND status IN ('claimed','running','cancelling')`, status, result, errorCode, update.UpdatedAt, update.UpdatedAt, runId, workflow, dbName, owner, runRevision); err != nil {
			return err
		}
		if err := releaseLease(ctx, connection, workflow, dbName, runId, owner, leaseRevision, update.UpdatedAt); err != nil {
			return err
		}
		run, err := requireRecord(loadRunRecord(ctx, connection, runId))
		if err != nil {
			return err
		}
		checkpoint, err := requireRecord(loadCheckpointRecord(ctx, connection, workflow, dbName))
		if err != nil {
			return err
		}
		lease, err := requireRecord(loadLeaseRecord(ctx, connection, workflow, dbName))
		if err != nil {
			return err
		}
		if run == nil || checkpoint == nil || lease == nil {
			return ErrNotFound
		}
		completed = RunFinalization{run, checkpoint, lease}
		return nil
	})
	if err != nil {
		return RunFinalization{}, err
	}
	return completed, nil
}

// FinalizeAttempt resolves every dedupe reservation and its subscriber item in one transaction.
func (repository *Repository) FinalizeAttempt(ctx context.Context, itemId int64, owner string, itemRevision int64, itemStatus ItemStatus, result, errorCode *string, runId int64, reservations []DedupeResolution, dedupeStatus DedupeStatus, message *string, now float64) (*RunItemRecord, error) {
	if err := validateAttemptFinalizationOwner(itemId, runId, owner, itemRevision, now); err != nil {
		return nil, err
	}
	if !itemStatus.IsTerminal() || dedupeStatus == DedupeStatusReserved || !dedupeStatus.valid() {
		return nil, invalid("Delivery attempt terminal status is invalid")
	}
	if err := validateOptionalJson(result); err != nil {
		return nil, err
	}
	if err := validateSymbol(errorCode, "Delivery item error code is invalid"); err != nil {
		return nil, err
	}
	if err := validateOptionalText(message, 256, "Delivery message id is invalid"); err != nil {
		return nil, err
	}
	if err := validateResolutions(reservations); err != nil {
		return nil, err
	}
	var item *RunItemRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		for _, reservation := range reservations {
			if err := resolveDedupe(ctx, connection, reservation.Id, runId, owner, reservation.ExpectedRevision, dedupeStatus, message, now); err != nil {
				return err
			}
		}
		if err := executeCas(ctx, connection, `UPDATE delivery_run_items SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND delivery_run_id=? AND owner_id=? AND revision=? AND status IN ('claimed','sending')`, itemStatus, result, errorCode, now, now, itemId, runId, owner, itemRevision); err != nil {
			return err
		}
		var err error
		item, err = requireRecord(loadRunItemRecord(ctx, connection, itemId))
		return err
	})
	return item, err
}

func validateRunFinalization(runId int64, owner string, runRevision int64, status RunStatus, result, errorCode *string, dbName string, checkpointRevision *int64, update CheckpointUpdate, leaseRevision int64) error {
	if err := validateRunFinalizationOwner(runId, owner, dbName, runRevision, update.UpdatedAt); err != nil {
		return err
	}
	if leaseRevision < 0 || checkpointRevision != nil && *checkpointRevision < 0 {
		return invalid("Delivery revision is invalid")
	}
	if !status.IsTerminal() {
		return invalid("Delivery terminal status is invalid")
	}
	if err := validateJson(update.SnapshotJson); err != nil {
		return err
	}
	if err := validateOptionalJson(result); err != nil {
		return err
	}
	if err := validateSymbol(errorCode, "Delivery error code is invalid"); err != nil {
		return err
	}
	return nil
}

func validateRunFinalizationOwner(runId int64, owner, dbName string, runRevision int64, now float64) error {
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return err
	}
	if err := validateIdentifier(owner, "Delivery owner id is invalid"); err != nil {
		return err
	}
	if err := validateDbName(dbName); err != nil {
		return err
	}
	if err := validateRevisionTime(runRevision, now); err != nil {
		return err
	}
	return nil
}

func validateAttemptFinalizationOwner(itemId, runId int64, owner string, itemRevision int64, now float64) error {
	if err := validatePositiveId(itemId, "Delivery item id is invalid"); err != nil {
		return err
	}
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return err
	}
	if err := validateIdentifier(owner, "Delivery item owner id is invalid"); err != nil {
		return err
	}
	if err := validateRevisionTime(itemRevision, now); err != nil {
		return err
	}
	return nil
}
