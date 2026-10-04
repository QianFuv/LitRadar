package delivery

import (
	"context"
	"database/sql"
)

// DedupeOutcome distinguishes a newly inserted reservation from an existing durable identity.
type DedupeOutcome struct {
	Kind   string        `json:"kind"`
	Record *DedupeRecord `json:"record"`
}

func validateDedupeScope(dbName string, userId, articleId int64) error {
	if err := validateDbName(dbName); err != nil {
		return err
	}
	if err := validatePositiveId(userId, "Delivery dedupe user id is invalid"); err != nil {
		return err
	}
	return validatePositiveId(articleId, "Delivery dedupe article id is invalid")
}

// LoadDedupe returns any prior reservation or terminal outcome for an article.
func (repository *Repository) LoadDedupe(ctx context.Context, workflow Workflow, dbName string, userId, articleId int64) (*DedupeRecord, error) {
	if err := validateDedupeScope(dbName, userId, articleId); err != nil {
		return nil, err
	}
	return scanDedupeRecord(repository.database.QueryRowContext(ctx, "SELECT "+dedupeColumns+" FROM delivery_dedupe WHERE workflow=? AND db_name=? AND user_id=? AND article_id=?", workflow, dbName, userId, articleId))
}

// ListDedupe returns the workflow's reservations in stable user/article order.
func (repository *Repository) ListDedupe(ctx context.Context, workflow Workflow, dbName string) ([]DedupeRecord, error) {
	if err := validateDbName(dbName); err != nil {
		return nil, err
	}
	rows, err := repository.database.QueryContext(ctx, "SELECT "+dedupeColumns+" FROM delivery_dedupe WHERE workflow=? AND db_name=? ORDER BY user_id,article_id", workflow, dbName)
	if err != nil {
		return nil, storageError(err)
	}
	defer rows.Close()
	result := []DedupeRecord{}
	for rows.Next() {
		record, err := scanDedupeRecord(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, *record)
	}
	return result, storageError(rows.Err())
}

// CleanupConfirmedDedupe never removes ambiguous or pending delivery protection.
func (repository *Repository) CleanupConfirmedDedupe(ctx context.Context, workflow Workflow, dbName string, before float64) (int, error) {
	if err := validateDbName(dbName); err != nil {
		return 0, err
	}
	if err := validateTime(before, "Delivery dedupe retention cutoff is invalid"); err != nil {
		return 0, err
	}
	result, err := repository.database.ExecContext(ctx, `DELETE FROM delivery_dedupe WHERE workflow=? AND db_name=? AND status='confirmed' AND delivered_at<?`, workflow, dbName, before)
	if err != nil {
		return 0, storageError(err)
	}
	count, err := result.RowsAffected()
	return int(count), storageError(err)
}

// ReserveDedupe inserts a unique identity after validating its run scope.
func (repository *Repository) ReserveDedupe(ctx context.Context, workflow Workflow, dbName string, userId, articleId, runId int64, owner string, now float64) (DedupeOutcome, error) {
	if err := validateDedupeScope(dbName, userId, articleId); err != nil {
		return DedupeOutcome{}, err
	}
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return DedupeOutcome{}, err
	}
	if err := validateIdentifier(owner, "Delivery dedupe owner id is invalid"); err != nil {
		return DedupeOutcome{}, err
	}
	if err := validateTime(now, "Delivery dedupe reservation time is invalid"); err != nil {
		return DedupeOutcome{}, err
	}
	var outcome DedupeOutcome
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		run, err := loadRunRecord(ctx, connection, runId)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.Workflow != workflow || run.DbName == nil || *run.DbName != dbName {
			return invalid("Delivery dedupe scope does not match its run")
		}
		result, err := connection.ExecContext(ctx, `INSERT INTO delivery_dedupe(workflow,db_name,user_id,article_id,delivery_run_id,status,message_id,reservation_owner,legacy_delivered_at,revision,reserved_at,delivered_at,updated_at) VALUES(?,?,?,?,?,'reserved',NULL,?,NULL,0,?,NULL,?) ON CONFLICT(workflow,db_name,user_id,article_id) DO NOTHING`, workflow, dbName, userId, articleId, runId, owner, now, now)
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		record, err := scanDedupeRecord(connection.QueryRowContext(ctx, "SELECT "+dedupeColumns+" FROM delivery_dedupe WHERE workflow=? AND db_name=? AND user_id=? AND article_id=?", workflow, dbName, userId, articleId))
		if err != nil {
			return err
		}
		if record == nil {
			return ErrNotFound
		}
		kind := "existing"
		if count == 1 {
			kind = "reserved"
		}
		outcome = DedupeOutcome{kind, record}
		return nil
	})
	return outcome, err
}

func resolveDedupe(ctx context.Context, connection *sql.Conn, id, runId int64, owner string, revision int64, status DedupeStatus, message *string, now float64) error {
	return executeCas(ctx, connection, `UPDATE delivery_dedupe SET status=?,message_id=?,reservation_owner=NULL,delivered_at=?,updated_at=?,revision=revision+1 WHERE id=? AND delivery_run_id=? AND reservation_owner=? AND revision=? AND status='reserved'`, status, message, now, now, id, runId, owner, revision)
}

// ResolveDedupe finalizes one reservation using owner and revision CAS.
func (repository *Repository) ResolveDedupe(ctx context.Context, id, runId int64, owner string, revision int64, status DedupeStatus, message *string, now float64) (*DedupeRecord, error) {
	if err := validatePositiveId(id, "Delivery dedupe id is invalid"); err != nil {
		return nil, err
	}
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return nil, err
	}
	if err := validateIdentifier(owner, "Delivery dedupe owner id is invalid"); err != nil {
		return nil, err
	}
	if err := validateRevisionTime(revision, now); err != nil {
		return nil, err
	}
	if status == DedupeStatusReserved || !status.valid() {
		return nil, invalid("Delivery dedupe terminal status is invalid")
	}
	if err := validateOptionalText(message, 256, "Delivery message id is invalid"); err != nil {
		return nil, err
	}
	var record *DedupeRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := resolveDedupe(ctx, connection, id, runId, owner, revision, status, message, now); err != nil {
			return err
		}
		var err error
		record, err = requireRecord(loadDedupeRecord(ctx, connection, id))
		return err
	})
	return record, err
}

// ReleaseReservations atomically releases only the exact pre-send identities owned by this attempt.
func (repository *Repository) ReleaseReservations(ctx context.Context, runId int64, owner string, reservations []DedupeResolution) (int, error) {
	if err := validatePositiveId(runId, "Delivery run id is invalid"); err != nil {
		return 0, err
	}
	if err := validateIdentifier(owner, "Delivery dedupe owner id is invalid"); err != nil {
		return 0, err
	}
	if err := validateResolutions(reservations); err != nil {
		return 0, err
	}
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		for _, reservation := range reservations {
			if err := executeCas(ctx, connection, `DELETE FROM delivery_dedupe WHERE id=? AND delivery_run_id=? AND reservation_owner=? AND revision=? AND status='reserved'`, reservation.Id, runId, owner, reservation.ExpectedRevision); err != nil {
				return err
			}
		}
		return nil
	})
	if err != nil {
		return 0, err
	}
	return len(reservations), nil
}
