package delivery

import (
	"context"
	"database/sql"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// LeaseOutcome distinguishes a newly acquired lease from an unexpired owner.
type LeaseOutcome struct {
	Kind  string       `json:"kind"`
	Lease *LeaseRecord `json:"lease"`
}

// LoadLease returns the persistent revision row even after release.
func (repository *Repository) LoadLease(ctx context.Context, workflow Workflow, dbName string) (*LeaseRecord, error) {
	if err := validateDBName(dbName); err != nil {
		return nil, err
	}
	return decodeLeaseRecord(queries.Lease(ctx, repository.database, deliverydb.LeaseParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName)}))
}

func validateLeaseScope(dbName string, runID int64, owner string) error {
	if err := validateDBName(dbName); err != nil {
		return err
	}
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
		return err
	}
	return validateIdentifier(owner, "Delivery lease owner id is invalid")
}

// AcquireLease acquires a free or expired scope; the same still-active owner also receives busy.
func (repository *Repository) AcquireLease(ctx context.Context, workflow Workflow, dbName string, runID int64, owner string, now, seconds float64) (LeaseOutcome, error) {
	if err := validateLeaseScope(dbName, runID, owner); err != nil {
		return LeaseOutcome{}, err
	}
	if err := validateLease(now, seconds); err != nil {
		return LeaseOutcome{}, err
	}
	var outcome LeaseOutcome
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		run, err := loadRunRecord(ctx, connection, runID)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.Workflow != workflow || run.DBName == nil || *run.DBName != dbName {
			return invalid("Delivery lease scope does not match its run")
		}
		existing, err := loadLeaseRecord(ctx, connection, workflow, dbName)
		if err != nil {
			return err
		}
		switch {
		case existing == nil:
			_, err = queries.CreateLease(ctx, connection, deliverydb.CreateLeaseParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), DeliveryRunID: new(sqlite.Integer(runID)), OwnerID: new(sqlite.Text(owner)), AcquiredAt: new(sqlite.Number(now)), HeartbeatAt: new(sqlite.Number(now)), ExpiresAt: new(sqlite.Number(now + seconds)), UpdatedAt: sqlite.Number(now)})
		case existing.OwnerID == nil || existing.ExpiresAt != nil && *existing.ExpiresAt <= now:
			err = checkCAS(queries.AcquireLease(ctx, connection, deliverydb.AcquireLeaseParams{DeliveryRunID: new(sqlite.Integer(runID)), OwnerID: new(sqlite.Text(owner)), AcquiredAt: new(sqlite.Number(now)), HeartbeatAt: new(sqlite.Number(now)), ExpiresAt: new(sqlite.Number(now + seconds)), UpdatedAt: sqlite.Number(now), ID: sqlite.Integer(existing.ID), Revision: sqlite.Integer(existing.Revision)}))
		default:

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
func (repository *Repository) RenewLease(ctx context.Context, workflow Workflow, dbName string, runID int64, owner string, revision int64, now, seconds float64) (*LeaseRecord, error) {
	if err := validateLeaseScope(dbName, runID, owner); err != nil {
		return nil, err
	}
	if err := validateRevisionLease(revision, now, seconds); err != nil {
		return nil, err
	}
	var record *LeaseRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := checkCAS(queries.RenewLease(ctx, connection, deliverydb.RenewLeaseParams{HeartbeatAt: new(sqlite.Number(now)), ExpiresAt: new(sqlite.Number(now + seconds)), UpdatedAt: sqlite.Number(now), Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), DeliveryRunID: new(sqlite.Integer(runID)), OwnerID: new(sqlite.Text(owner)), Revision: sqlite.Integer(revision)})); err != nil {
			return err
		}
		var err error
		record, err = requireRecord(loadLeaseRecord(ctx, connection, workflow, dbName))
		return err
	})
	return record, err
}

// ReleaseLease clears ownership while retaining and advancing the monotonic revision.
func (repository *Repository) ReleaseLease(ctx context.Context, workflow Workflow, dbName string, runID int64, owner string, revision int64, now float64) (*LeaseRecord, error) {
	if err := validateLeaseScope(dbName, runID, owner); err != nil {
		return nil, err
	}
	if err := validateRevisionTime(revision, now); err != nil {
		return nil, err
	}
	var record *LeaseRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		if err := releaseLease(ctx, connection, workflow, dbName, runID, owner, revision, now); err != nil {
			return err
		}
		var err error
		record, err = requireRecord(loadLeaseRecord(ctx, connection, workflow, dbName))
		return err
	})
	return record, err
}

func releaseLease(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string, runID int64, owner string, revision int64, now float64) error {
	return checkCAS(queries.ReleaseLease(ctx, connection, deliverydb.ReleaseLeaseParams{UpdatedAt: sqlite.Number(now), Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), DeliveryRunID: new(sqlite.Integer(runID)), OwnerID: new(sqlite.Text(owner)), Revision: sqlite.Integer(revision)}))
}

// ReconcileAfterTakeover quarantines all sending work before releasing safe pre-send reservations.
func (repository *Repository) ReconcileAfterTakeover(ctx context.Context, runID int64, owner string, revision int64, now float64) (RecoveryResult, error) {
	if err := validatePositiveID(runID, "Delivery run id is invalid"); err != nil {
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
		run, err := loadRunRecord(ctx, connection, runID)
		if err != nil {
			return err
		}
		if run == nil {
			return ErrNotFound
		}
		if run.OwnerID == nil || *run.OwnerID != owner || run.Revision != revision || !run.Status.IsActive() || run.LeaseExpiresAt == nil || *run.LeaseExpiresAt <= now {
			return ErrConflict
		}
		{
			executed, err := queries.QuarantineDedupe(ctx, connection, deliverydb.QuarantineDedupeParams{DeliveredAt: new(sqlite.Number(now)), UpdatedAt: sqlite.Number(now), DeliveryRunID: new(sqlite.Integer(runID))})
			if err != nil {
				return err
			}
			count, err := executed.RowsAffected()
			if err != nil {
				return err
			}
			result.UnknownDedupeCount = int(count)
		}
		{
			executed, err := queries.QuarantineItems(ctx, connection, deliverydb.QuarantineItemsParams{UpdatedAt: sqlite.Number(now), FinishedAt: new(sqlite.Number(now)), DeliveryRunID: sqlite.Integer(runID)})
			if err != nil {
				return err
			}
			count, err := executed.RowsAffected()
			if err != nil {
				return err
			}
			result.UnknownItemCount = int(count)
		}
		{
			executed, err := queries.ReleaseDedupe(ctx, connection, deliverydb.ReleaseDedupeParams{DeliveryRunID: new(sqlite.Integer(runID))})
			if err != nil {
				return err
			}
			count, err := executed.RowsAffected()
			if err != nil {
				return err
			}
			result.ReleasedDedupeCount = int(count)
		}
		{
			executed, err := queries.ResetClaimedItems(ctx, connection, deliverydb.ResetClaimedItemsParams{UpdatedAt: sqlite.Number(now), DeliveryRunID: sqlite.Integer(runID)})
			if err != nil {
				return err
			}
			count, err := executed.RowsAffected()
			if err != nil {
				return err
			}
			result.ResetItemCount = int(count)
		}
		return nil
	})
	return result, err
}
