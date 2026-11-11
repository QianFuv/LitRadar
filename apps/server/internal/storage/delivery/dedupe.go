package delivery

import (
	"context"
	"database/sql"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// DedupeOutcome distinguishes a newly inserted reservation from an existing durable identity.
type DedupeOutcome struct {
	Kind   string        `json:"kind"`
	Record *DedupeRecord `json:"record"`
}

func validateDedupeScope(dbName string, userID, articleID int64) error {
	if err := validateDBName(dbName); err != nil {
		return err
	}
	if err := validatePositiveID(userID, "Delivery dedupe user id is invalid"); err != nil {
		return err
	}
	return validatePositiveID(articleID, "Delivery dedupe article id is invalid")
}

// LoadDedupe returns any prior reservation or terminal outcome for an article.
func (repository *Repository) LoadDedupe(ctx context.Context, workflow Workflow, dbName string, userID, articleID int64) (*DedupeRecord, error) {
	if err := validateDedupeScope(dbName, userID, articleID); err != nil {
		return nil, err
	}
	return decodeDedupeRecord(queries.Dedupe(ctx, repository.database, deliverydb.DedupeParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), UserID: sqlite.Integer(userID), ArticleID: sqlite.Integer(articleID)}))
}

// ListDedupe returns the workflow's reservations in stable user/article order.
func (repository *Repository) ListDedupe(ctx context.Context, workflow Workflow, dbName string) ([]DedupeRecord, error) {
	if err := validateDBName(dbName); err != nil {
		return nil, err
	}
	rows, err := queries.DedupeList(ctx, repository.database, deliverydb.DedupeListParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName)})
	if err != nil {
		return nil, storageError(err)
	}
	result := make([]DedupeRecord, 0, len(rows))
	for _, row := range rows {
		record, err := decodeDedupeRecord(row, nil)
		if err != nil {
			return nil, err
		}
		result = append(result, *record)
	}
	return result, nil
}

// CleanupConfirmedDedupe never removes ambiguous or pending delivery protection.
func (repository *Repository) CleanupConfirmedDedupe(ctx context.Context, workflow Workflow, dbName string, before float64) (int, error) {
	if err := validateDBName(dbName); err != nil {
		return 0, err
	}
	if err := validateTime(before, "Delivery dedupe retention cutoff is invalid"); err != nil {
		return 0, err
	}
	result, err := queries.CleanupDedupe(ctx, repository.database, deliverydb.CleanupDedupeParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), DeliveredAt: new(sqlite.Number(before))})
	if err != nil {
		return 0, storageError(err)
	}
	count, err := result.RowsAffected()
	return int(count), storageError(err)
}

// ReserveDedupe inserts a unique identity after validating its run scope.
func (repository *Repository) ReserveDedupe(ctx context.Context, workflow Workflow, dbName string, userID, articleID, runID int64, owner string, now float64) (DedupeOutcome, error) {
	if err := validateDedupeScope(dbName, userID, articleID); err != nil {
		return DedupeOutcome{}, err
	}
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
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
		run, err := loadRunRecord(ctx, connection, runID)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.Workflow != workflow || run.DBName == nil || *run.DBName != dbName {
			return invalid("Delivery dedupe scope does not match its run")
		}
		result, err := queries.ReserveDedupe(ctx, connection, deliverydb.ReserveDedupeParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), UserID: sqlite.Integer(userID), ArticleID: sqlite.Integer(articleID), DeliveryRunID: new(sqlite.Integer(runID)), ReservationOwner: new(sqlite.Text(owner)), ReservedAt: sqlite.Number(now), UpdatedAt: sqlite.Number(now)})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		record, err := decodeDedupeRecord(queries.Dedupe(ctx, connection, deliverydb.DedupeParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), UserID: sqlite.Integer(userID), ArticleID: sqlite.Integer(articleID)}))
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

func resolveDedupe(ctx context.Context, connection *sql.Conn, id, runID int64, owner string, revision int64, status DedupeStatus, message *string, now float64) error {
	return checkCAS(queries.ResolveDedupe(ctx, connection, deliverydb.ResolveDedupeParams{Status: sqlite.Text(status), MessageID: (*sqlite.Text)(message), DeliveredAt: new(sqlite.Number(now)), UpdatedAt: sqlite.Number(now), ID: sqlite.Integer(id), DeliveryRunID: new(sqlite.Integer(runID)), ReservationOwner: new(sqlite.Text(owner)), Revision: sqlite.Integer(revision)}))
}

// ResolveDedupe finalizes one reservation using owner and revision CAS.
func (repository *Repository) ResolveDedupe(ctx context.Context, id, runID int64, owner string, revision int64, status DedupeStatus, message *string, now float64) (*DedupeRecord, error) {
	if err := validatePositiveID(id, "Delivery dedupe id is invalid"); err != nil {
		return nil, err
	}
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
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
		if err := resolveDedupe(ctx, connection, id, runID, owner, revision, status, message, now); err != nil {
			return err
		}
		var err error
		record, err = requireRecord(loadDedupeRecord(ctx, connection, id))
		return err
	})
	return record, err
}

// ReleaseReservations atomically releases only the exact pre-send identities owned by this attempt.
func (repository *Repository) ReleaseReservations(ctx context.Context, runID int64, owner string, reservations []DedupeResolution) (int, error) {
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
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
			if err := checkCAS(queries.ReleaseReservation(ctx, connection, deliverydb.ReleaseReservationParams{ID: sqlite.Integer(reservation.ID), DeliveryRunID: new(sqlite.Integer(runID)), ReservationOwner: new(sqlite.Text(owner)), Revision: sqlite.Integer(reservation.ExpectedRevision)})); err != nil {
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
