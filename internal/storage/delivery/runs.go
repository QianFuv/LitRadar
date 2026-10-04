package delivery

import (
	"context"
	"database/sql"
)

// RunOutcome distinguishes successful admission/claim from idempotent, busy and unavailable observations.
type RunOutcome struct {
	Kind string     `json:"kind"`
	Run  *RunRecord `json:"run"`
}

// AdmitRun atomically inserts a queued run or returns the original identity/active manual owner.
func (repository *Repository) AdmitRun(ctx context.Context, run RunCreate) (RunOutcome, error) {
	if err := validateRunCreate(run); err != nil {
		return RunOutcome{}, err
	}
	var outcome RunOutcome
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		var err error
		outcome, err = admitRun(ctx, connection, run)
		return err
	})
	return outcome, err
}

// EnqueueRun requires a new admission and treats an existing or busy identity as conflict.
func (repository *Repository) EnqueueRun(ctx context.Context, run RunCreate) (*RunRecord, error) {
	outcome, err := repository.AdmitRun(ctx, run)
	if err != nil {
		return nil, err
	}
	if outcome.Kind != "enqueued" {
		return nil, ErrConflict
	}
	return outcome.Run, nil
}

func admitRun(ctx context.Context, connection *sql.Conn, run RunCreate) (RunOutcome, error) {
	result, err := connection.ExecContext(ctx, `INSERT INTO delivery_runs(external_id,workflow,scope_key,db_name,trigger_kind,mode,user_id,status,legacy_status,owner_id,lease_expires_at,deadline_at,cancellation_requested,result_json,error_code,revision,created_at,started_at,updated_at,finished_at) VALUES(?,?,?,?,?,?,?,'queued',NULL,NULL,NULL,?,0,NULL,NULL,0,?,NULL,?,NULL) ON CONFLICT DO NOTHING`, run.ExternalId, run.Workflow, run.ScopeKey, run.DbName, run.TriggerKind, run.Mode, run.UserId, run.DeadlineAt, run.CreatedAt, run.CreatedAt)
	if err != nil {
		return RunOutcome{}, storageError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return RunOutcome{}, storageError(err)
	}
	if count == 1 {
		id, err := result.LastInsertId()
		if err != nil {
			return RunOutcome{}, storageError(err)
		}
		record, err := requireRecord(loadRunRecord(ctx, connection, id))
		return RunOutcome{"enqueued", record}, err
	}
	record, err := scanRunRecord(connection.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE workflow=? AND scope_key=? AND external_id=?", run.Workflow, run.ScopeKey, run.ExternalId))
	if err != nil {
		return RunOutcome{}, err
	}
	if record != nil {
		return RunOutcome{"existing", record}, nil
	}
	if run.TriggerKind == TriggerKindManual {
		record, err = scanRunRecord(connection.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? AND status IN ('queued','claimed','running','cancelling') ORDER BY id LIMIT 1", run.UserId))
		if err != nil {
			return RunOutcome{}, err
		}
		if record != nil {
			return RunOutcome{"busy", record}, nil
		}
	}
	return RunOutcome{}, ErrConflict
}

// LoadRun reads a durable run by its positive row identifier.
func (repository *Repository) LoadRun(ctx context.Context, id int64) (*RunRecord, error) {
	if err := validatePositiveId(id, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	return scanRunRecord(repository.database.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE id=?", id))
}

// ClaimRun claims queued work or takes over expired active work after checking competing database owners.
func (repository *Repository) ClaimRun(ctx context.Context, id int64, owner string, revision int64, now, seconds float64) (RunOutcome, error) {
	if err := validatePositiveId(id, "Delivery run id is invalid"); err != nil {
		return RunOutcome{}, err
	}
	if err := validateIdentifier(owner, "Delivery owner id is invalid"); err != nil {
		return RunOutcome{}, err
	}
	if err := validateRevisionLease(revision, now, seconds); err != nil {
		return RunOutcome{}, err
	}
	var outcome RunOutcome
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		run, err := loadRunRecord(ctx, connection, id)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.Revision != revision {
			return ErrConflict
		}
		canClaim := run.Status == RunStatusQueued || run.Status.IsActive() && run.LeaseExpiresAt != nil && *run.LeaseExpiresAt <= now
		if !canClaim {
			outcome = RunOutcome{"unavailable", run}
			return nil
		}
		if run.DbName != nil {
			active, err := scanRunRecord(connection.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE id<>? AND workflow=? AND db_name=? AND status IN ('claimed','running','cancelling') ORDER BY id LIMIT 1", id, run.Workflow, *run.DbName))
			if err != nil {
				return err
			}
			if active != nil {
				outcome = RunOutcome{"busy", active}
				return nil
			}
		}
		status := RunStatusClaimed
		if run.CancellationRequested {
			status = RunStatusCancelling
		}
		if err := executeCas(ctx, connection, `UPDATE delivery_runs SET status=?,owner_id=?,lease_expires_at=?,started_at=COALESCE(started_at,?),updated_at=?,revision=revision+1 WHERE id=? AND revision=?`, status, owner, now+seconds, now, now, id, revision); err != nil {
			return err
		}
		run, err = requireRecord(loadRunRecord(ctx, connection, id))
		outcome = RunOutcome{"claimed", run}
		return err
	})
	return outcome, err
}

func validateRunOwner(id int64, owner string, revision int64, now float64) error {
	if err := validatePositiveId(id, "Delivery run id is invalid"); err != nil {
		return err
	}
	if err := validateIdentifier(owner, "Delivery owner id is invalid"); err != nil {
		return err
	}
	return validateRevisionTime(revision, now)
}

func (repository *Repository) updateRun(ctx context.Context, id int64, query string, args ...any) (*RunRecord, error) {
	var run *RunRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := executeCas(ctx, connection, query, args...); err != nil {
			return err
		}
		var err error
		run, err = requireRecord(loadRunRecord(ctx, connection, id))
		if err == nil && run == nil {
			return ErrNotFound
		}
		return err
	})
	return run, err
}

// RenewRun extends an active unexpired owner and advances its revision.
func (repository *Repository) RenewRun(ctx context.Context, id int64, owner string, revision int64, now, seconds float64) (*RunRecord, error) {
	if err := validateRunOwner(id, owner, revision, now); err != nil {
		return nil, err
	}
	if err := validateLease(now, seconds); err != nil {
		return nil, err
	}
	return repository.updateRun(ctx, id, `UPDATE delivery_runs SET lease_expires_at=?1,updated_at=?2,revision=revision+1 WHERE id=?3 AND owner_id=?4 AND revision=?5 AND status IN ('claimed','running','cancelling') AND lease_expires_at>?2`, now+seconds, now, id, owner, revision)
}

// StartRun transitions an unexpired claimed owner to running.
func (repository *Repository) StartRun(ctx context.Context, id int64, owner string, revision int64, now float64) (*RunRecord, error) {
	if err := validateRunOwner(id, owner, revision, now); err != nil {
		return nil, err
	}
	return repository.updateRun(ctx, id, `UPDATE delivery_runs SET status='running',updated_at=?1,revision=revision+1 WHERE id=?2 AND owner_id=?3 AND revision=?4 AND status='claimed' AND lease_expires_at>?1`, now, id, owner, revision)
}

// CancelRun finalizes queued work or asks an active owner to drain cancellation.
func (repository *Repository) CancelRun(ctx context.Context, id, revision int64, now float64) (*RunRecord, error) {
	if err := validatePositiveId(id, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	if err := validateRevisionTime(revision, now); err != nil {
		return nil, err
	}
	var run *RunRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		current, err := loadRunRecord(ctx, connection, id)
		if err != nil {
			return err
		}
		if current == nil {
			return ErrNotFound
		}
		if current.Revision != revision || current.Status.IsTerminal() {
			return ErrConflict
		}
		status := RunStatusCancelling
		var finished *float64
		if current.Status == RunStatusQueued {
			status = RunStatusCancelled
			finished = &now
		} else if !current.Status.IsActive() {
			return ErrConflict
		}
		if err := executeCas(ctx, connection, `UPDATE delivery_runs SET status=?,cancellation_requested=1,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND revision=?`, status, now, finished, id, revision); err != nil {
			return err
		}
		run, err = requireRecord(loadRunRecord(ctx, connection, id))
		return err
	})
	return run, err
}

// FinalizeRun commits a known terminal state using owner/revision CAS, including an expired owner that has not been replaced.
func (repository *Repository) FinalizeRun(ctx context.Context, id int64, owner string, revision int64, status RunStatus, result, errorCode *string, now float64) (*RunRecord, error) {
	if err := validateRunOwner(id, owner, revision, now); err != nil {
		return nil, err
	}
	if !status.IsTerminal() {
		return nil, invalid("Delivery terminal status is invalid")
	}
	if err := validateOptionalJson(result); err != nil {
		return nil, err
	}
	if err := validateSymbol(errorCode, "Delivery error code is invalid"); err != nil {
		return nil, err
	}
	return repository.updateRun(ctx, id, `UPDATE delivery_runs SET status=?,owner_id=NULL,lease_expires_at=NULL,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND owner_id=? AND revision=? AND status IN ('claimed','running','cancelling')`, status, result, errorCode, now, now, id, owner, revision)
}

// FinalizeQueuedRun records a dispatcher failure only while the observed run remains queued.
func (repository *Repository) FinalizeQueuedRun(ctx context.Context, id, revision int64, status RunStatus, result, errorCode *string, now float64) (*RunRecord, error) {
	if err := validatePositiveId(id, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	if err := validateRevisionTime(revision, now); err != nil {
		return nil, err
	}
	if status != RunStatusFailed && status != RunStatusCancelled && status != RunStatusTimedOut {
		return nil, invalid("Queued delivery terminal status is invalid")
	}
	if err := validateOptionalJson(result); err != nil {
		return nil, err
	}
	if err := validateSymbol(errorCode, "Delivery error code is invalid"); err != nil {
		return nil, err
	}
	return repository.updateRun(ctx, id, `UPDATE delivery_runs SET status=?,result_json=?,error_code=?,updated_at=?,finished_at=?,revision=revision+1 WHERE id=? AND revision=? AND status='queued'`, status, result, errorCode, now, now, id, revision)
}
