package delivery

import (
	"context"
	"database/sql"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

func latestManual(ctx context.Context, connection *sql.Conn, userID int64) (*RunRecord, error) {
	return decodeRunRecord(queries.LatestManualRun(ctx, connection, deliverydb.LatestManualRunParams{UserID: new(sqlite.Integer(userID))}))
}

// AdmitManualRun checks unknown quarantine and normal admission in the same write transaction.
func (repository *Repository) AdmitManualRun(ctx context.Context, run RunCreate) (RunOutcome, error) {
	if err := validateRunCreate(run); err != nil {
		return RunOutcome{}, err
	}
	if run.TriggerKind != TriggerKindManual {
		return RunOutcome{}, invalid("Manual delivery admission requires a manual trigger")
	}
	var outcome RunOutcome
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		latest, err := latestManual(ctx, connection, *run.UserID)
		if err != nil {
			return err
		}
		if latest != nil && latest.Status == RunStatusUnknown {
			outcome = RunOutcome{"blocked_unknown", latest}
			return nil
		}
		outcome, err = admitRun(ctx, connection, run)
		return err
	})
	return outcome, err
}

// AcknowledgeUnknownManualRun preserves the old ambiguous run and atomically admits and audits one matching replacement.
func (repository *Repository) AcknowledgeUnknownManualRun(ctx context.Context, userID int64, unknownExternalID string, replacement RunCreate, audit *domain.AuditEvent) (*RunRecord, error) {
	if err := validatePositiveID(userID, "Manual delivery user id is invalid"); err != nil {
		return nil, err
	}
	if err := validateIdentifier(unknownExternalID, "Manual delivery acknowledgement job id is invalid"); err != nil {
		return nil, err
	}
	if err := validateRunCreate(replacement); err != nil {
		return nil, err
	}
	if replacement.TriggerKind != TriggerKindManual || replacement.UserID == nil || *replacement.UserID != userID {
		return nil, invalid("Manual delivery acknowledgement requires a replacement for the same user")
	}
	var record *RunRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		unknown, err := decodeRunRecord(queries.ManualRun(ctx, connection, deliverydb.ManualRunParams{UserID: new(sqlite.Integer(userID)), ExternalID: sqlite.Text(unknownExternalID)}))
		if err != nil {
			return err
		}
		if unknown == nil {
			return ErrNotFound
		}
		latest, err := latestManual(ctx, connection, userID)
		if err != nil {
			return err
		}
		if latest == nil {
			return ErrNotFound
		}
		if latest.ID != unknown.ID || unknown.Status != RunStatusUnknown || replacement.Workflow != unknown.Workflow || replacement.ScopeKey != unknown.ScopeKey || !sameOptionalString(replacement.DBName, unknown.DBName) || replacement.Mode != unknown.Mode {
			return ErrConflict
		}
		outcome, err := admitRun(ctx, connection, replacement)
		if err != nil {
			return err
		}
		if outcome.Kind != "enqueued" {
			return ErrConflict
		}
		if audit == nil {
			return &Error{Kind: "audit"}
		}
		event := *audit
		event.ActorID = &userID
		event.TargetID = &unknown.ID
		if err := auth.InsertAudit(ctx, connection, &event); err != nil {
			return &Error{Kind: "audit", cause: err}
		}
		record = outcome.Run
		return nil
	})
	return record, err
}

func sameOptionalString(left, right *string) bool {
	return left == nil && right == nil || left != nil && right != nil && *left == *right
}

// LoadLatestManualRun selects by row identifier, not timestamp or terminal status.
func (repository *Repository) LoadLatestManualRun(ctx context.Context, userID int64) (*RunRecord, error) {
	if err := validatePositiveID(userID, "Manual delivery user id is invalid"); err != nil {
		return nil, err
	}
	return decodeRunRecord(queries.LatestManualRun(ctx, repository.database, deliverydb.LatestManualRunParams{UserID: new(sqlite.Integer(userID))}))
}

// LoadManualRun restricts the opaque external identifier to its authenticated user.
func (repository *Repository) LoadManualRun(ctx context.Context, userID int64, externalID string) (*RunRecord, error) {
	if err := validatePositiveID(userID, "Manual delivery user id is invalid"); err != nil {
		return nil, err
	}
	if err := validateIdentifier(externalID, "Manual delivery job id is invalid"); err != nil {
		return nil, err
	}
	return decodeRunRecord(queries.ManualRun(ctx, repository.database, deliverydb.ManualRunParams{UserID: new(sqlite.Integer(userID)), ExternalID: sqlite.Text(externalID)}))
}

// LoadManualRunForAdmin reads an opaque job after the caller has established administrator authority.
func (repository *Repository) LoadManualRunForAdmin(ctx context.Context, externalID string) (*RunRecord, error) {
	if err := validateIdentifier(externalID, "Manual delivery job id is invalid"); err != nil {
		return nil, err
	}
	return decodeRunRecord(queries.AdminManualRun(ctx, repository.database, deliverydb.AdminManualRunParams{ExternalID: sqlite.Text(externalID)}))
}

// ListDispatchableManualRuns bounds pending and expired active work in stable creation/id order.
func (repository *Repository) ListDispatchableManualRuns(ctx context.Context, now float64, limit int) ([]RunRecord, error) {
	if err := validateTime(now, "Manual delivery dispatch time is invalid"); err != nil {
		return nil, err
	}
	if limit < 1 || limit > 64 {
		return nil, invalid("Manual delivery dispatch limit is invalid")
	}
	rows, err := queries.DispatchableManualRuns(ctx, repository.database, deliverydb.DispatchableManualRunsParams{LeaseExpiresAt: new(sqlite.Number(now)), Limit: int64(limit)})
	if err != nil {
		return nil, storageError(err)
	}
	result := make([]RunRecord, 0, len(rows))
	for _, row := range rows {
		record, err := decodeRunRecord(row, nil)
		if err != nil {
			return nil, err
		}
		result = append(result, *record)
	}
	return result, nil
}
