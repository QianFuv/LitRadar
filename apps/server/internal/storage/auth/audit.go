package auth

import (
	"context"
	"database/sql"
	"errors"
	"log/slog"
	"math"
	"sync/atomic"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/authdb"
	sqlite3 "modernc.org/sqlite/lib"
)

var ErrAuditEvent = errors.New("Security audit event is invalid")
var ErrAuditRetention = errors.New("Security audit retention days are invalid")
var auditFailureCount atomic.Uint64

// AuditRecord is an ordered offline projection of persisted fixed-schema metadata.
type AuditRecord struct {
	Id int64
	domain.AuditEvent
}

// RetentionResult records a bounded batch and whether expired backlog remains.
type RetentionResult struct {
	DidRun         bool
	DeletedCount   int64
	HasMoreExpired bool
	Cutoff         float64
}

// AuditFailureCount reads the process-local number of failed durable audit operations.
func AuditFailureCount() uint64 { return auditFailureCount.Load() }

// ReportAuditFailure logs a fixed classification, never the original error or request values.
func ReportAuditFailure(kind string) uint64 {
	return ReportAuditFailureContext(context.Background(), kind)
}

// ReportAuditFailureContext retains the originating scope while reporting fixed classifications.
func ReportAuditFailureContext(ctx context.Context, kind string) uint64 {
	switch kind {
	case "sqlite", "io", "invalid_event", "invalid_retention_days", "join", "executor_unavailable":
	default:
		kind = "execution"
	}
	count := auditFailureCount.Add(1)
	if count == 0 {
		count = math.MaxUint64
	}
	slog.ErrorContext(ctx, "audit.persistence_failed", "event", "audit.persistence_failed", "component", "security", "outcome", "failure", "error_kind", kind, "failure_count", count)
	return count
}

// IsTransientContention recognizes busy/locked SQLite failures even inside an audit error wrapper.
func IsTransientContention(err error) bool {
	var failure interface{ Code() int }
	return errors.As(err, &failure) && (failure.Code()&255 == sqlite3.SQLITE_BUSY || failure.Code()&255 == sqlite3.SQLITE_LOCKED)
}

// AppendAudit commits a standalone terminal event in its own immediate transaction.
func (repository *Repository) AppendAudit(ctx context.Context, event domain.AuditEvent) (int64, error) {
	var id int64
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		if !validAudit(&event) {
			return ErrAuditEvent
		}
		if err := insertAudit(ctx, connection, &event); err != nil {
			return err
		}
		return func() error {
			queryResult, err := queries.LastInsertID(ctx, connection)
			if err != nil {
				return err
			}
			id = queryResult
			return nil
		}()
	})
	if err != nil {
		recordAuditError(ctx, err)
		return 0, safeAuditError(err)
	}
	return id, nil
}

// ListAudit returns insertion order for trusted offline consumers.
func (repository *Repository) ListAudit(ctx context.Context) ([]AuditRecord, error) {
	rows, err := queries.AuditEvents(ctx, repository.database)
	if err != nil {
		return nil, AuditFailure{cause: err}
	}
	result := make([]AuditRecord, 0, len(rows))
	for _, row := range rows {
		result = append(result, AuditRecord{Id: row.ID, AuditEvent: domain.AuditEvent{ActorId: row.ActorID, TargetId: row.TargetID, Action: row.Action, Outcome: row.Outcome, Reason: row.Reason, RequestId: row.RequestID, SourceClass: row.SourceClass, Bucket: row.Bucket, RejectedCount: row.RejectedCount, RetryAfterSeconds: row.RetryAfterSeconds, OccurredAt: row.OccurredAt}})
	}
	return result, nil
}

// CleanupAudit deletes at most 10,000 expired rows, advancing the daily marker only after drainage.
func (repository *Repository) CleanupAudit(ctx context.Context, retentionDays uint32, now float64) (RetentionResult, error) {
	if retentionDays < 1 || retentionDays > 3650 || math.IsNaN(now) || math.IsInf(now, 0) {
		recordAuditError(ctx, ErrAuditRetention)
		return RetentionResult{}, ErrAuditRetention
	}
	result := RetentionResult{Cutoff: now - float64(retentionDays)*86400}
	err := repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		var last *float64
		if err := func() error {
			queryResult, err := queries.AuditRetention(ctx, connection)
			if err != nil {
				return err
			}
			last = queryResult
			return nil
		}(); err != nil {
			return AuditFailure{cause: err}
		}
		if last != nil && *last >= now-86400 {
			return nil
		}
		deleted, err := queries.PruneAuditEvents(ctx, connection, authdb.PruneAuditEventsParams{OccurredAt: result.Cutoff})
		if err != nil {
			return AuditFailure{cause: err}
		}
		result.DeletedCount, err = deleted.RowsAffected()
		if err != nil {
			return AuditFailure{cause: err}
		}
		if err := func() error {
			queryResult, err := queries.HasExpiredAuditEvents(ctx, connection, authdb.HasExpiredAuditEventsParams{OccurredAt: result.Cutoff})
			if err != nil {
				return err
			}
			result.HasMoreExpired = queryResult
			return nil
		}(); err != nil {
			return AuditFailure{cause: err}
		}
		if !result.HasMoreExpired {
			updated, err := queries.RecordAuditRetention(ctx, connection, authdb.RecordAuditRetentionParams{LastRetentionAt: new(now)})
			if err != nil {
				return AuditFailure{cause: err}
			}
			count, err := updated.RowsAffected()
			if err != nil {
				return AuditFailure{cause: err}
			}
			if count != 1 {
				return AuditFailure{cause: sql.ErrNoRows}
			}
		}
		result.DidRun = true
		return nil
	})
	if err != nil {
		recordAuditError(ctx, err)
		return RetentionResult{}, safeAuditError(err)
	}
	return result, nil
}

func safeAuditError(err error) error {
	if errors.Is(err, ErrAuditEvent) || errors.Is(err, ErrAuditRetention) || errors.Is(err, domain.ErrAudit) {
		return err
	}
	return AuditFailure{cause: err}
}

func recordAuditError(ctx context.Context, err error) {
	kind := "sqlite"
	if errors.Is(err, ErrAuditEvent) {
		kind = "invalid_event"
	}
	if errors.Is(err, ErrAuditRetention) {
		kind = "invalid_retention_days"
	}
	ReportAuditFailureContext(ctx, kind)
}
