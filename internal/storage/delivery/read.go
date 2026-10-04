package delivery

import (
	"context"
	"database/sql"
	"errors"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type rowScanner interface{ Scan(...any) error }

const checkpointColumns = `id, workflow, db_name, status, legacy_status, snapshot_json, last_completed_run_at,
     revision, legacy_source_hash, legacy_source_name, legacy_imported_at, created_at, updated_at`

func scanCheckpointRecord(row rowScanner) (*CheckpointRecord, error) {
	var result CheckpointRecord
	var column0 sqlite.Integer
	var column1 enumScanner[Workflow]
	var column2 sqlite.Text
	var column3 enumScanner[CheckpointStatus]
	var column4 sqlite.OptionalText
	var column5 sqlite.Text
	var column6 sqlite.OptionalText
	var column7 sqlite.Integer
	var column8 sqlite.OptionalText
	var column9 sqlite.OptionalText
	var column10 optionalNumber
	var column11 sqlite.Number
	var column12 sqlite.Number
	if err := row.Scan(&column0, &column1, &column2, &column3, &column4, &column5, &column6, &column7, &column8, &column9, &column10, &column11, &column12); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		return nil, storageError(err)
	}
	result.Id = int64(column0)
	result.Workflow = column1.value
	result.DbName = string(column2)
	result.Status = column3.value
	result.LegacyStatus = column4.Value
	result.SnapshotJson = string(column5)
	result.LastCompletedRunAt = column6.Value
	result.Revision = int64(column7)
	result.LegacySourceHash = column8.Value
	result.LegacySourceName = column9.Value
	result.LegacyImportedAt = column10.Value
	result.CreatedAt = float64(column11)
	result.UpdatedAt = float64(column12)
	return &result, nil
}

const runColumns = `id, external_id, workflow, scope_key, db_name, trigger_kind, mode, user_id, status,
     legacy_status, owner_id, lease_expires_at, deadline_at, cancellation_requested,
     result_json, error_code, revision, created_at, started_at, updated_at, finished_at`

func scanRunRecord(row rowScanner) (*RunRecord, error) {
	var result RunRecord
	var column0 sqlite.Integer
	var column1 sqlite.Text
	var column2 enumScanner[Workflow]
	var column3 sqlite.Text
	var column4 sqlite.OptionalText
	var column5 enumScanner[TriggerKind]
	var column6 enumScanner[RunMode]
	var column7 sqlite.OptionalInteger
	var column8 enumScanner[RunStatus]
	var column9 sqlite.OptionalText
	var column10 sqlite.OptionalText
	var column11 optionalNumber
	var column12 optionalNumber
	var column13 sqlite.Integer
	var column14 sqlite.OptionalText
	var column15 sqlite.OptionalText
	var column16 sqlite.Integer
	var column17 sqlite.Number
	var column18 optionalNumber
	var column19 sqlite.Number
	var column20 optionalNumber
	if err := row.Scan(&column0, &column1, &column2, &column3, &column4, &column5, &column6, &column7, &column8, &column9, &column10, &column11, &column12, &column13, &column14, &column15, &column16, &column17, &column18, &column19, &column20); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		return nil, storageError(err)
	}
	result.Id = int64(column0)
	result.ExternalId = string(column1)
	result.Workflow = column2.value
	result.ScopeKey = string(column3)
	result.DbName = column4.Value
	result.TriggerKind = column5.value
	result.Mode = column6.value
	result.UserId = column7.Value
	result.Status = column8.value
	result.LegacyStatus = column9.Value
	result.OwnerId = column10.Value
	result.LeaseExpiresAt = column11.Value
	result.DeadlineAt = column12.Value
	result.CancellationRequested = column13 != 0
	result.ResultJson = column14.Value
	result.ErrorCode = column15.Value
	result.Revision = int64(column16)
	result.CreatedAt = float64(column17)
	result.StartedAt = column18.Value
	result.UpdatedAt = float64(column19)
	result.FinishedAt = column20.Value
	return &result, nil
}

const itemColumns = `id, delivery_run_id, item_kind, item_key, user_id, article_id, status, legacy_status,
     owner_id, lease_expires_at, attempt_count, result_json, error_code, revision,
     created_at, started_at, updated_at, finished_at`

func scanRunItemRecord(row rowScanner) (*RunItemRecord, error) {
	var result RunItemRecord
	var column0 sqlite.Integer
	var column1 sqlite.Integer
	var column2 enumScanner[ItemKind]
	var column3 sqlite.Text
	var column4 sqlite.OptionalInteger
	var column5 sqlite.OptionalInteger
	var column6 enumScanner[ItemStatus]
	var column7 sqlite.OptionalText
	var column8 sqlite.OptionalText
	var column9 optionalNumber
	var column10 sqlite.Integer
	var column11 sqlite.OptionalText
	var column12 sqlite.OptionalText
	var column13 sqlite.Integer
	var column14 sqlite.Number
	var column15 optionalNumber
	var column16 sqlite.Number
	var column17 optionalNumber
	if err := row.Scan(&column0, &column1, &column2, &column3, &column4, &column5, &column6, &column7, &column8, &column9, &column10, &column11, &column12, &column13, &column14, &column15, &column16, &column17); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		return nil, storageError(err)
	}
	result.Id = int64(column0)
	result.DeliveryRunId = int64(column1)
	result.ItemKind = column2.value
	result.ItemKey = string(column3)
	result.UserId = column4.Value
	result.ArticleId = column5.Value
	result.Status = column6.value
	result.LegacyStatus = column7.Value
	result.OwnerId = column8.Value
	result.LeaseExpiresAt = column9.Value
	result.AttemptCount = int64(column10)
	result.ResultJson = column11.Value
	result.ErrorCode = column12.Value
	result.Revision = int64(column13)
	result.CreatedAt = float64(column14)
	result.StartedAt = column15.Value
	result.UpdatedAt = float64(column16)
	result.FinishedAt = column17.Value
	return &result, nil
}

const dedupeColumns = `id, workflow, db_name, user_id, article_id, delivery_run_id, status, message_id,
     reservation_owner, legacy_delivered_at, revision, reserved_at, delivered_at, updated_at`

func scanDedupeRecord(row rowScanner) (*DedupeRecord, error) {
	var result DedupeRecord
	var column0 sqlite.Integer
	var column1 enumScanner[Workflow]
	var column2 sqlite.Text
	var column3 sqlite.Integer
	var column4 sqlite.Integer
	var column5 sqlite.OptionalInteger
	var column6 enumScanner[DedupeStatus]
	var column7 sqlite.OptionalText
	var column8 sqlite.OptionalText
	var column9 sqlite.OptionalText
	var column10 sqlite.Integer
	var column11 sqlite.Number
	var column12 optionalNumber
	var column13 sqlite.Number
	if err := row.Scan(&column0, &column1, &column2, &column3, &column4, &column5, &column6, &column7, &column8, &column9, &column10, &column11, &column12, &column13); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		return nil, storageError(err)
	}
	result.Id = int64(column0)
	result.Workflow = column1.value
	result.DbName = string(column2)
	result.UserId = int64(column3)
	result.ArticleId = int64(column4)
	result.DeliveryRunId = column5.Value
	result.Status = column6.value
	result.MessageId = column7.Value
	result.ReservationOwner = column8.Value
	result.LegacyDeliveredAt = column9.Value
	result.Revision = int64(column10)
	result.ReservedAt = float64(column11)
	result.DeliveredAt = column12.Value
	result.UpdatedAt = float64(column13)
	return &result, nil
}

const leaseColumns = `id, workflow, db_name, delivery_run_id, owner_id, revision, acquired_at,
     heartbeat_at, expires_at, updated_at`

func scanLeaseRecord(row rowScanner) (*LeaseRecord, error) {
	var result LeaseRecord
	var column0 sqlite.Integer
	var column1 enumScanner[Workflow]
	var column2 sqlite.Text
	var column3 sqlite.OptionalInteger
	var column4 sqlite.OptionalText
	var column5 sqlite.Integer
	var column6 optionalNumber
	var column7 optionalNumber
	var column8 optionalNumber
	var column9 sqlite.Number
	if err := row.Scan(&column0, &column1, &column2, &column3, &column4, &column5, &column6, &column7, &column8, &column9); err != nil {
		if errors.Is(err, sql.ErrNoRows) {
			return nil, nil
		}
		return nil, storageError(err)
	}
	result.Id = int64(column0)
	result.Workflow = column1.value
	result.DbName = string(column2)
	result.DeliveryRunId = column3.Value
	result.OwnerId = column4.Value
	result.Revision = int64(column5)
	result.AcquiredAt = column6.Value
	result.HeartbeatAt = column7.Value
	result.ExpiresAt = column8.Value
	result.UpdatedAt = float64(column9)
	return &result, nil
}

type storedEnum interface {
	~string
	valid() bool
}
type enumScanner[T storedEnum] struct{ value T }

func (scanner *enumScanner[T]) Scan(source any) error {
	var text sqlite.Text
	if err := text.Scan(source); err != nil {
		return err
	}
	scanner.value = T(text)
	if !scanner.value.valid() {
		return &Error{Kind: "sqlite"}
	}
	return nil
}

type optionalNumber struct{ Value *float64 }

func (number *optionalNumber) Scan(source any) error {
	if source == nil {
		number.Value = nil
		return nil
	}
	var parsed sqlite.Number
	if err := parsed.Scan(source); err != nil {
		return err
	}
	value := float64(parsed)
	number.Value = &value
	return nil
}

func loadRunRecord(ctx context.Context, connection *sql.Conn, id int64) (*RunRecord, error) {
	return scanRunRecord(connection.QueryRowContext(ctx, "SELECT "+runColumns+" FROM delivery_runs WHERE id=?", id))
}

func loadRunItemRecord(ctx context.Context, connection *sql.Conn, id int64) (*RunItemRecord, error) {
	return scanRunItemRecord(connection.QueryRowContext(ctx, "SELECT "+itemColumns+" FROM delivery_run_items WHERE id=?", id))
}

func loadDedupeRecord(ctx context.Context, connection *sql.Conn, id int64) (*DedupeRecord, error) {
	return scanDedupeRecord(connection.QueryRowContext(ctx, "SELECT "+dedupeColumns+" FROM delivery_dedupe WHERE id=?", id))
}

func loadCheckpointRecord(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string) (*CheckpointRecord, error) {
	return scanCheckpointRecord(connection.QueryRowContext(ctx, "SELECT "+checkpointColumns+" FROM delivery_checkpoints WHERE workflow=? AND db_name=?", workflow, dbName))
}

func loadLeaseRecord(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string) (*LeaseRecord, error) {
	return scanLeaseRecord(connection.QueryRowContext(ctx, "SELECT "+leaseColumns+" FROM delivery_leases WHERE workflow=? AND db_name=?", workflow, dbName))
}
