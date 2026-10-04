package delivery

import (
	"context"
	"database/sql"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
)

func latestManual(ctx context.Context, connection *sql.Conn, userId int64) (*RunRecord, error) {
	return scanRunRecord(connection.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? ORDER BY id DESC LIMIT 1", userId))
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
		latest, err := latestManual(ctx, connection, *run.UserId)
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
func (repository *Repository) AcknowledgeUnknownManualRun(ctx context.Context, userId int64, unknownExternalId string, replacement RunCreate, audit *domain.AuditEvent) (*RunRecord, error) {
	if err := validatePositiveId(userId, "Manual delivery user id is invalid"); err != nil {
		return nil, err
	}
	if err := validateIdentifier(unknownExternalId, "Manual delivery acknowledgement job id is invalid"); err != nil {
		return nil, err
	}
	if err := validateRunCreate(replacement); err != nil {
		return nil, err
	}
	if replacement.TriggerKind != TriggerKindManual || replacement.UserId == nil || *replacement.UserId != userId {
		return nil, invalid("Manual delivery acknowledgement requires a replacement for the same user")
	}
	var record *RunRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		unknown, err := scanRunRecord(connection.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? AND external_id=? ORDER BY id DESC LIMIT 1", userId, unknownExternalId))
		if err != nil {
			return err
		}
		if unknown == nil {
			return ErrNotFound
		}
		latest, err := latestManual(ctx, connection, userId)
		if err != nil {
			return err
		}
		if latest == nil {
			return ErrNotFound
		}
		if latest.Id != unknown.Id || unknown.Status != RunStatusUnknown || replacement.Workflow != unknown.Workflow || replacement.ScopeKey != unknown.ScopeKey || !sameOptionalString(replacement.DbName, unknown.DbName) || replacement.Mode != unknown.Mode {
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
		event.ActorId = &userId
		event.TargetId = &unknown.Id
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
func (repository *Repository) LoadLatestManualRun(ctx context.Context, userId int64) (*RunRecord, error) {
	if err := validatePositiveId(userId, "Manual delivery user id is invalid"); err != nil {
		return nil, err
	}
	return scanRunRecord(repository.database.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? ORDER BY id DESC LIMIT 1", userId))
}

// LoadManualRun restricts the opaque external identifier to its authenticated user.
func (repository *Repository) LoadManualRun(ctx context.Context, userId int64, externalId string) (*RunRecord, error) {
	if err := validatePositiveId(userId, "Manual delivery user id is invalid"); err != nil {
		return nil, err
	}
	if err := validateIdentifier(externalId, "Manual delivery job id is invalid"); err != nil {
		return nil, err
	}
	return scanRunRecord(repository.database.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND user_id=? AND external_id=? ORDER BY id DESC LIMIT 1", userId, externalId))
}

// LoadManualRunForAdmin reads an opaque job after the caller has established administrator authority.
func (repository *Repository) LoadManualRunForAdmin(ctx context.Context, externalId string) (*RunRecord, error) {
	if err := validateIdentifier(externalId, "Manual delivery job id is invalid"); err != nil {
		return nil, err
	}
	return scanRunRecord(repository.database.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND external_id=? ORDER BY id DESC LIMIT 1", externalId))
}

// ListDispatchableManualRuns bounds pending and expired active work in stable creation/id order.
func (repository *Repository) ListDispatchableManualRuns(ctx context.Context, now float64, limit int) ([]RunRecord, error) {
	if err := validateTime(now, "Manual delivery dispatch time is invalid"); err != nil {
		return nil, err
	}
	if limit < 1 || limit > 64 {
		return nil, invalid("Manual delivery dispatch limit is invalid")
	}
	rows, err := repository.database.QueryContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE trigger_kind='manual' AND (status='queued' OR (status IN ('claimed','running','cancelling') AND lease_expires_at<=?)) ORDER BY created_at,id LIMIT ?", now, limit)
	if err != nil {
		return nil, storageError(err)
	}
	defer rows.Close()
	result := []RunRecord{}
	for rows.Next() {
		record, err := scanRunRecord(rows)
		if err != nil {
			return nil, err
		}
		result = append(result, *record)
	}
	return result, storageError(rows.Err())
}
