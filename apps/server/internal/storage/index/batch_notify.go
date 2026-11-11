package index

import (
	"context"
	"database/sql"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/batchdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// PrepareNotifyAttempt reuses running identity, blocks ambiguity and records explicit Unknown acknowledgement before retry.
func PrepareNotifyAttempt(ctx context.Context, connection *sql.Conn, id, owner string, ordinal uint64, newAttempt string, shouldAcknowledge bool, now int64) (NotifyAttemptPreparation, error) {
	if err := validateNotifyAttemptInput(newAttempt, now); err != nil {
		return NotifyAttemptPreparation{}, err
	}
	var result NotifyAttemptPreparation
	err := immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		phase, err := readCatalogPhase(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		if phase != CatalogNotifying {
			return batchState("notification attempt requires the notifying phase")
		}
		current, err := readNotifyHandoff(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		state := NotifyHandoffState{AttemptId: newAttempt, Status: NotifyRunning}
		decision := "run"
		shouldWrite := true
		switch {
		case current == nil:
		case current.Status == NotifyRunning:
			state = *current
			shouldWrite = false
		case current.Status.IsSuccess():
			state = *current
			decision = "succeeded"
			shouldWrite = false
		case current.Status.canRetry():
			if current.AttemptId == newAttempt {
				return batchInput("notification retry attempt identifier must be new")
			}
			state.UnknownAcknowledgedAttemptId = current.UnknownAcknowledgedAttemptId
			state.UnknownAcknowledgedAt = current.UnknownAcknowledgedAt
		case !shouldAcknowledge:
			state = *current
			decision = "blocked_unknown"
			shouldWrite = false
		default:
			if current.AttemptId == newAttempt {
				return batchInput("notification acknowledged attempt identifier must be new")
			}
			state.UnknownAcknowledgedAttemptId = &current.AttemptId
			state.UnknownAcknowledgedAt = &now
		}
		if shouldWrite {
			if err := saveNotifyHandoff(ctx, connection, id, ordinal, state, now); err != nil {
				return err
			}
			if err := touchBatch(ctx, connection, id, now); err != nil {
				return err
			}
		}
		result = NotifyAttemptPreparation{decision, state}
		return nil
	})
	return result, err
}

// RecordNotifyAttemptResult accepts observations only for the current stable attempt and immutable terminal replay.
func RecordNotifyAttemptResult(ctx context.Context, connection *sql.Conn, id, owner string, ordinal uint64, attempt string, status NotifyStatus, exit *int32, now int64) (NotifyHandoffState, error) {
	if err := validateNotifyAttemptInput(attempt, now); err != nil {
		return NotifyHandoffState{}, err
	}
	var result NotifyHandoffState
	err := immediate(ctx, connection, func() error {
		if err := verifyBatchOwnership(ctx, connection, id, owner, now); err != nil {
			return err
		}
		phase, err := readCatalogPhase(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		if phase != CatalogNotifying {
			return batchState("notification result requires the notifying phase")
		}
		current, err := readNotifyHandoff(ctx, connection, id, ordinal)
		if err != nil {
			return err
		}
		if current == nil {
			return batchState("notification result has no prepared attempt")
		}
		if current.AttemptId != attempt {
			return batchState("notification result attempt identifier is stale")
		}
		if current.Status != NotifyRunning {
			if current.Status == status && sameOptional(current.ExitCode, exit) {
				result = *current
				return nil
			}
			return batchState("notification attempt already has a different terminal result")
		}
		current.Status = status
		current.ExitCode = copyValue(exit)
		if err := saveNotifyHandoff(ctx, connection, id, ordinal, *current, now); err != nil {
			return err
		}
		if err := touchBatch(ctx, connection, id, now); err != nil {
			return err
		}
		result = *current
		return nil
	})
	return result, err
}
func validateNotifyAttemptInput(attempt string, now int64) error {
	if err := validateIdentifier(attempt, "notification attempt identifier must be non-empty and bounded"); err != nil {
		return err
	}
	if now < 0 {
		return batchInput("notification attempt timestamp must not be negative")
	}
	return nil
}
func saveNotifyHandoff(ctx context.Context, connection *sql.Conn, id string, ordinal uint64, state NotifyHandoffState, now int64) error {
	if err := validateNotifyHandoff(state); err != nil {
		return err
	}
	count, err := sqliteCount(ordinal)
	if err != nil {
		return err
	}
	changed, err := batchQueries.StoreNotifyHandoff(ctx, connection, batchdb.StoreNotifyHandoffParams{BatchID: sqlite.Text(id), Ordinal: sqlite.Integer(count), NotifyAttemptID: new(sqlite.Text(state.AttemptId)), NotifyStatus: new(sqlite.Text(state.Status)), NotifyExitCode: func() *sqlite.Integer {
		if state.ExitCode == nil {
			return nil
		}
		return new(sqlite.Integer(*state.ExitCode))
	}(), NotifyUnknownAcknowledgedAttemptID: (*sqlite.Text)(state.UnknownAcknowledgedAttemptId), NotifyUnknownAcknowledgedAt: (*sqlite.Integer)(state.UnknownAcknowledgedAt), UpdatedAt: sqlite.Integer(now)})
	return expectBatchChange(changed, err, "batch catalog row is missing")
}
