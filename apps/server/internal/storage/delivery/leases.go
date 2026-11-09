package delivery

import (
	"context"
	"database/sql"
)

// LeaseOutcome distinguishes a newly acquired lease from an unexpired owner.
type LeaseOutcome struct {
	Kind  string       `json:"kind"`
	Lease *LeaseRecord `json:"lease"`
}

// LoadLease returns the persistent revision row even after release.
func (repository *Repository) LoadLease(ctx context.Context, workflow Workflow, dbName string) (*LeaseRecord, error) {
	if err := validateDbName(dbName); err != nil {
		return nil, err
	}
	return scanLeaseRecord(repository.database.QueryRowContext(ctx, "SELECT "+leaseColumns+" FROM delivery_leases WHERE workflow=? AND db_name=?", workflow, dbName))
}

func validateLeaseScope(dbName string, runId int64, owner string) error {
	if err := validateDbName(dbName); err != nil {
		return err
	}
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return err
	}
	return validateIdentifier(owner, "Delivery lease owner id is invalid")
}

// AcquireLease acquires a free or expired scope; the same still-active owner also receives busy.
func (repository *Repository) AcquireLease(ctx context.Context, workflow Workflow, dbName string, runId int64, owner string, now, seconds float64) (LeaseOutcome, error) {
	if err := validateLeaseScope(dbName, runId, owner); err != nil {
		return LeaseOutcome{}, err
	}
	if err := validateLease(now, seconds); err != nil {
		return LeaseOutcome{}, err
	}
	var outcome LeaseOutcome
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		run, err := loadRunRecord(ctx, connection, runId)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.Workflow != workflow || run.DbName == nil || *run.DbName != dbName {
			return invalid("Delivery lease scope does not match its run")
		}
		existing, err := loadLeaseRecord(ctx, connection, workflow, dbName)
		if err != nil {
			return err
		}
		if existing == nil {
			_, err = connection.ExecContext(ctx, `INSERT INTO delivery_leases(workflow,db_name,delivery_run_id,owner_id,revision,acquired_at,heartbeat_at,expires_at,updated_at) VALUES(?,?,?,?,0,?,?,?,?)`, workflow, dbName, runId, owner, now, now, now+seconds, now)
		} else if existing.OwnerId == nil || existing.ExpiresAt != nil && *existing.ExpiresAt <= now {
			err = executeCas(ctx, connection, `UPDATE delivery_leases SET delivery_run_id=?,owner_id=?,revision=revision+1,acquired_at=?,heartbeat_at=?,expires_at=?,updated_at=? WHERE id=? AND revision=?`, runId, owner, now, now, now+seconds, now, existing.Id, existing.Revision)
		} else {
			outcome = LeaseOutcome{"busy", existing}
			return nil
		}
		if err != nil {
			return err
		}
		record, err := requireRecord(loadLeaseRecord(ctx, connection, workflow, dbName))
		outcome = LeaseOutcome{"acquired", record}
		return err
	})
	return outcome, err
}

// RenewLease requires the exact unexpired owner, run and revision.
func (repository *Repository) RenewLease(ctx context.Context, workflow Workflow, dbName string, runId int64, owner string, revision int64, now, seconds float64) (*LeaseRecord, error) {
	if err := validateLeaseScope(dbName, runId, owner); err != nil {
		return nil, err
	}
	if err := validateRevisionLease(revision, now, seconds); err != nil {
		return nil, err
	}
	var record *LeaseRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := executeCas(ctx, connection, `UPDATE delivery_leases SET heartbeat_at=?1,expires_at=?2,updated_at=?3,revision=revision+1 WHERE workflow=?4 AND db_name=?5 AND delivery_run_id=?6 AND owner_id=?7 AND revision=?8 AND expires_at>?1`, now, now+seconds, now, workflow, dbName, runId, owner, revision); err != nil {
			return err
		}
		var err error
		record, err = requireRecord(loadLeaseRecord(ctx, connection, workflow, dbName))
		return err
	})
	return record, err
}

// ReleaseLease clears ownership while retaining and advancing the monotonic revision.
func (repository *Repository) ReleaseLease(ctx context.Context, workflow Workflow, dbName string, runId int64, owner string, revision int64, now float64) (*LeaseRecord, error) {
	if err := validateLeaseScope(dbName, runId, owner); err != nil {
		return nil, err
	}
	if err := validateRevisionTime(revision, now); err != nil {
		return nil, err
	}
	var record *LeaseRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := releaseLease(ctx, connection, workflow, dbName, runId, owner, revision, now); err != nil {
			return err
		}
		var err error
		record, err = requireRecord(loadLeaseRecord(ctx, connection, workflow, dbName))
		return err
	})
	return record, err
}

func releaseLease(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string, runId int64, owner string, revision int64, now float64) error {
	return executeCas(ctx, connection, `UPDATE delivery_leases SET delivery_run_id=NULL,owner_id=NULL,acquired_at=NULL,heartbeat_at=NULL,expires_at=NULL,updated_at=?,revision=revision+1 WHERE workflow=? AND db_name=? AND delivery_run_id=? AND owner_id=? AND revision=?`, now, workflow, dbName, runId, owner, revision)
}

// ReconcileAfterTakeover quarantines all sending work before releasing safe pre-send reservations.
func (repository *Repository) ReconcileAfterTakeover(ctx context.Context, runId int64, owner string, revision int64, now float64) (RecoveryResult, error) {
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return RecoveryResult{}, err
	}
	if err := validateIdentifier(owner, "Delivery run owner id is invalid"); err != nil {
		return RecoveryResult{}, err
	}
	if err := validateRevisionTime(revision, now); err != nil {
		return RecoveryResult{}, err
	}
	var result RecoveryResult
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		run, err := loadRunRecord(ctx, connection, runId)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.OwnerId == nil || *run.OwnerId != owner || run.Revision != revision || !run.Status.IsActive() || run.LeaseExpiresAt == nil || *run.LeaseExpiresAt <= now {
			return ErrConflict
		}
		steps := []struct {
			query string
			args  []any
			count *int
		}{
			{`UPDATE delivery_dedupe SET status='unknown',reservation_owner=NULL,delivered_at=?1,updated_at=?2,revision=revision+1 WHERE delivery_run_id=?3 AND status='reserved' AND user_id IN (SELECT user_id FROM delivery_run_items WHERE delivery_run_id=?3 AND item_kind='subscriber' AND status='sending' AND user_id IS NOT NULL)`, []any{now, now, runId}, &result.UnknownDedupeCount},
			{`UPDATE delivery_run_items SET status='unknown',owner_id=NULL,lease_expires_at=NULL,error_code='abandoned_sending',updated_at=?,finished_at=?,revision=revision+1 WHERE delivery_run_id=? AND status='sending'`, []any{now, now, runId}, &result.UnknownItemCount},
			{`DELETE FROM delivery_dedupe WHERE delivery_run_id=? AND status='reserved'`, []any{runId}, &result.ReleasedDedupeCount},
			{`UPDATE delivery_run_items SET status='pending',owner_id=NULL,lease_expires_at=NULL,updated_at=?,revision=revision+1 WHERE delivery_run_id=? AND status='claimed'`, []any{now, runId}, &result.ResetItemCount},
		}
		for _, step := range steps {
			executed, err := connection.ExecContext(ctx, step.query, step.args...)
			if err != nil {
				return err
			}
			count, err := executed.RowsAffected()
			if err != nil {
				return err
			}
			*step.count = int(count)
		}
		return nil
	})
	return result, err
}
