package index

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"

	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func loadBatch(ctx context.Context, connection *sql.Conn, id, owner string, didResume bool) (IndexBatch, error) {
	var started sqlite.Integer
	err := connection.QueryRowContext(ctx, `SELECT started_at FROM index_batches WHERE batch_id=?1`, id).Scan(&started)
	if errors.Is(err, sql.ErrNoRows) {
		return IndexBatch{}, batchState("batch row is missing")
	}
	if err != nil {
		return IndexBatch{}, err
	}
	catalogs, err := ReadBatchCatalogs(ctx, connection, id)
	return IndexBatch{id, owner, int64(started), didResume, catalogs}, err
}

// ReadBatchCatalogs loads typed persisted metadata in original ordinal order before validating recovery payloads.
func ReadBatchCatalogs(ctx context.Context, connection *sql.Conn, id string) ([]IndexBatchCatalog, error) {
	rows, err := connection.QueryContext(ctx, `SELECT ordinal,file_name,catalog_name,provider_name,journal_count,phase FROM index_batch_catalogs WHERE batch_id=?1 ORDER BY ordinal`, id)
	if err != nil {
		return nil, err
	}
	type rawCatalog struct {
		ordinal, count              sqlite.Integer
		file, name, provider, phase sqlite.Text
	}
	var raw []rawCatalog
	for rows.Next() {
		var value rawCatalog
		if err := rows.Scan(&value.ordinal, &value.file, &value.name, &value.provider, &value.count, &value.phase); err != nil {
			rows.Close()
			return nil, err
		}
		raw = append(raw, value)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return nil, err
	}
	catalogs := make([]IndexBatchCatalog, 0, len(raw))
	for _, value := range raw {
		ordinal, err := storedCount(int64(value.ordinal))
		if err != nil {
			return nil, err
		}
		count, err := storedCount(int64(value.count))
		if err != nil {
			return nil, err
		}
		phase, err := parseCatalogPhase(string(value.phase))
		if err != nil {
			return nil, err
		}
		outcome, err := readCatalogOutcome(ctx, connection, id, ordinal)
		if err != nil {
			return nil, err
		}
		intent, err := readManifestIntent(ctx, connection, id, ordinal)
		if err != nil {
			return nil, err
		}
		handoff, err := readNotifyHandoff(ctx, connection, id, ordinal)
		if err != nil {
			return nil, err
		}
		catalogs = append(catalogs, IndexBatchCatalog{ordinal, string(value.file), string(value.name), string(value.provider), count, phase, outcome, intent, handoff})
	}
	return catalogs, nil
}
func parseCatalogPhase(value string) (CatalogPhase, error) {
	phase := CatalogPhase(value)
	switch phase {
	case CatalogPending, CatalogIndexing, CatalogManifestPrepared, CatalogManifestPublished, CatalogNotifying, CatalogCompleted:
		return phase, nil
	}
	return "", batchState("stored catalog phase is invalid")
}
func parseNotifyStatus(value string) (NotifyStatus, error) {
	status := NotifyStatus(value)
	switch status {
	case NotifyRunning, NotifyIdle, NotifyCompleted, NotifySkipped, NotifyFailed, NotifyCancelled, NotifyTimedOut, NotifyUnknown:
		return status, nil
	}
	return "", batchState("stored notification handoff status is invalid")
}
func readCatalogPhase(ctx context.Context, connection *sql.Conn, id string, ordinal uint64) (CatalogPhase, error) {
	count, err := sqliteCount(ordinal)
	if err != nil {
		return "", err
	}
	var phase sqlite.Text
	err = connection.QueryRowContext(ctx, `SELECT phase FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2`, id, count).Scan(&phase)
	if errors.Is(err, sql.ErrNoRows) {
		return "", batchState("batch catalog row is missing")
	}
	if err != nil {
		return "", err
	}
	return parseCatalogPhase(string(phase))
}

type optionalBatchBlob struct {
	value    []byte
	hasValue bool
}

func (value *optionalBatchBlob) Scan(source any) error {
	if source == nil {
		value.value = nil
		value.hasValue = false
		return nil
	}
	bytes, ok := source.([]byte)
	if !ok {
		return errors.New("invalid SQLite blob column")
	}
	value.value = append([]byte{}, bytes...)
	value.hasValue = true
	return nil
}
func readManifestIntent(ctx context.Context, connection *sql.Conn, id string, ordinal uint64) (*ManifestIntent, error) {
	count, err := sqliteCount(ordinal)
	if err != nil {
		return nil, err
	}
	var payload optionalBatchBlob
	var digest, path, run, generated sqlite.OptionalText
	var through sqlite.OptionalInteger
	err = connection.QueryRowContext(ctx, `SELECT manifest_payload,manifest_sha256,manifest_through_event_id,manifest_path,manifest_run_id,manifest_generated_at FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2`, id, count).Scan(&payload, &digest, &through, &path, &run, &generated)
	if err != nil {
		return nil, err
	}
	if !payload.hasValue && digest.Value == nil && through.Value == nil && path.Value == nil && run.Value == nil && generated.Value == nil {
		return nil, nil
	}
	if !payload.hasValue || digest.Value == nil || path.Value == nil || run.Value == nil || generated.Value == nil {
		return nil, batchState("stored manifest intent is incomplete")
	}
	intent := ManifestIntent{payload.value, *digest.Value, through.Value, *path.Value, *run.Value, *generated.Value}
	if err := validateManifestIntent(intent); err != nil {
		return nil, err
	}
	return &intent, nil
}
func readCatalogOutcome(ctx context.Context, connection *sql.Conn, id string, ordinal uint64) (*BatchCatalogOutcome, error) {
	count, err := sqliteCount(ordinal)
	if err != nil {
		return nil, err
	}
	var run, path sqlite.OptionalText
	var journals sqlite.Integer
	var written, attempts sqlite.OptionalInteger
	err = connection.QueryRowContext(ctx, `SELECT run_id,journal_count,written_article_count,source_attempt_count,outcome_manifest_path FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2`, id, count).Scan(&run, &journals, &written, &attempts, &path)
	if err != nil {
		return nil, err
	}
	if run.Value == nil {
		if written.Value != nil || attempts.Value != nil || path.Value != nil {
			return nil, batchState("stored catalog outcome is incomplete")
		}
		return nil, nil
	}
	journalCount, err := storedCount(int64(journals))
	if err != nil {
		return nil, err
	}
	if written.Value == nil || attempts.Value == nil {
		return nil, batchState("stored catalog outcome is incomplete")
	}
	attemptCount, err := storedCount(*attempts.Value)
	if err != nil {
		return nil, err
	}
	outcome := BatchCatalogOutcome{*run.Value, journalCount, *written.Value, attemptCount, path.Value}
	if err := validateCatalogOutcome(outcome); err != nil {
		return nil, err
	}
	return &outcome, nil
}

type batchExitRangeError struct{ value int64 }

func (err *batchExitRangeError) Error() string {
	return fmt.Sprintf("Integer %d out of range at index 2", err.value)
}

type optionalBatchExit struct{ value *int32 }

func (value *optionalBatchExit) Scan(source any) error {
	var integer sqlite.OptionalInteger
	if err := integer.Scan(source); err != nil {
		return err
	}
	value.value = nil
	if integer.Value != nil {
		if *integer.Value < math.MinInt32 || *integer.Value > math.MaxInt32 {
			return &batchExitRangeError{*integer.Value}
		}
		converted := int32(*integer.Value)
		value.value = &converted
	}
	return nil
}

func readNotifyHandoff(ctx context.Context, connection *sql.Conn, id string, ordinal uint64) (*NotifyHandoffState, error) {
	count, err := sqliteCount(ordinal)
	if err != nil {
		return nil, err
	}
	var attempt, status, ackAttempt sqlite.OptionalText
	var exit optionalBatchExit
	var ackTime sqlite.OptionalInteger
	err = connection.QueryRowContext(ctx, `SELECT notify_attempt_id,notify_status,notify_exit_code,notify_unknown_acknowledged_attempt_id,notify_unknown_acknowledged_at FROM index_batch_catalogs WHERE batch_id=?1 AND ordinal=?2`, id, count).Scan(&attempt, &status, &exit, &ackAttempt, &ackTime)
	if err != nil {
		var rangeError *batchExitRangeError
		if errors.As(err, &rangeError) {
			return nil, rangeError
		}
		return nil, err
	}
	code := exit.value
	if attempt.Value == nil && status.Value == nil && code == nil && ackAttempt.Value == nil && ackTime.Value == nil {
		return nil, nil
	}
	if attempt.Value == nil || status.Value == nil {
		return nil, batchState("stored notification handoff is incomplete")
	}
	parsed, err := parseNotifyStatus(*status.Value)
	if err != nil {
		return nil, err
	}
	state := NotifyHandoffState{*attempt.Value, parsed, code, ackAttempt.Value, ackTime.Value}
	if err := validateNotifyHandoff(state); err != nil {
		return nil, err
	}
	return &state, nil
}

func validateManifestIntent(intent ManifestIntent) error {
	if len(intent.Payload) == 0 || len(intent.Payload) > 64*1024*1024 {
		return batchState("stored manifest payload is empty or too large")
	}
	if len(intent.Sha256) != 64 || sha256Hex(intent.Payload) != intent.Sha256 {
		return batchState("stored manifest payload digest is invalid")
	}
	if intent.ThroughEventId != nil && *intent.ThroughEventId <= 0 {
		return batchState("stored manifest outbox cursor is invalid")
	}
	if err := validateRelativePath(intent.Path); err != nil {
		return err
	}
	if err := validateIdentifier(intent.RunId, "manifest run identifier must be non-empty and bounded"); err != nil {
		return err
	}
	return validateIdentifier(intent.GeneratedAt, "manifest timestamp must be non-empty and bounded")
}
func validateCatalogOutcome(outcome BatchCatalogOutcome) error {
	if err := validateIdentifier(outcome.RunId, "catalog run identifier must be non-empty and bounded"); err != nil {
		return err
	}
	if outcome.WrittenArticleCount < 0 {
		return batchInput("written article count must not be negative")
	}
	if outcome.ManifestPath != nil {
		return validateRelativePath(*outcome.ManifestPath)
	}
	return nil
}
func validateNotifyHandoff(state NotifyHandoffState) error {
	if err := validateIdentifier(state.AttemptId, "notification attempt identifier must be non-empty and bounded"); err != nil {
		return err
	}
	hasZero := state.ExitCode != nil && *state.ExitCode == 0
	if state.Status.IsSuccess() && !hasZero {
		return batchState("successful notification handoff must have a zero exit code")
	}
	if state.Status != NotifyUnknown && hasZero && !state.Status.IsSuccess() {
		return batchState("unsuccessful notification handoff cannot have a zero exit code")
	}
	if (state.UnknownAcknowledgedAttemptId == nil) != (state.UnknownAcknowledgedAt == nil) {
		return batchState("stored notification acknowledgement is incomplete")
	}
	if state.UnknownAcknowledgedAttemptId != nil {
		if err := validateIdentifier(*state.UnknownAcknowledgedAttemptId, "acknowledged notification attempt identifier must be non-empty and bounded"); err != nil {
			return err
		}
		if *state.UnknownAcknowledgedAt < 0 {
			return batchState("notification acknowledgement timestamp must not be negative")
		}
	}
	return nil
}
