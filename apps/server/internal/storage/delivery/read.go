package delivery

import (
	"context"
	"database/sql"
	"errors"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

var queries deliverydb.Queries

func decodeCheckpointRecord(row deliverydb.DeliveryCheckpoint, err error) (*CheckpointRecord, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, storageError(err)
	}
	if !(Workflow(row.Workflow)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	if !(CheckpointStatus(row.Status)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	var result CheckpointRecord
	result.ID = int64(row.ID)
	result.Workflow = Workflow(row.Workflow)
	result.DBName = string(row.DBName)
	result.Status = CheckpointStatus(row.Status)
	result.LegacyStatus = (*string)(row.LegacyStatus)
	result.SnapshotJSON = string(row.SnapshotJSON)
	result.LastCompletedRunAt = (*string)(row.LastCompletedRunAt)
	result.Revision = int64(row.Revision)
	result.LegacySourceHash = (*string)(row.LegacySourceHash)
	result.LegacySourceName = (*string)(row.LegacySourceName)
	result.LegacyImportedAt = (*float64)(row.LegacyImportedAt)
	result.CreatedAt = float64(row.CreatedAt)
	result.UpdatedAt = float64(row.UpdatedAt)
	return &result, nil
}

func decodeRunRecord(row deliverydb.DeliveryRun, err error) (*RunRecord, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, storageError(err)
	}
	if !(Workflow(row.Workflow)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	if !(TriggerKind(row.TriggerKind)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	if !(RunMode(row.Mode)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	if !(RunStatus(row.Status)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	var result RunRecord
	result.ID = int64(row.ID)
	result.ExternalID = string(row.ExternalID)
	result.Workflow = Workflow(row.Workflow)
	result.ScopeKey = string(row.ScopeKey)
	result.DBName = (*string)(row.DBName)
	result.TriggerKind = TriggerKind(row.TriggerKind)
	result.Mode = RunMode(row.Mode)
	result.UserID = (*int64)(row.UserID)
	result.Status = RunStatus(row.Status)
	result.LegacyStatus = (*string)(row.LegacyStatus)
	result.OwnerID = (*string)(row.OwnerID)
	result.LeaseExpiresAt = (*float64)(row.LeaseExpiresAt)
	result.DeadlineAt = (*float64)(row.DeadlineAt)
	result.CancellationRequested = row.CancellationRequested != 0
	result.ResultJSON = (*string)(row.ResultJSON)
	result.ErrorCode = (*string)(row.ErrorCode)
	result.Revision = int64(row.Revision)
	result.CreatedAt = float64(row.CreatedAt)
	result.StartedAt = (*float64)(row.StartedAt)
	result.UpdatedAt = float64(row.UpdatedAt)
	result.FinishedAt = (*float64)(row.FinishedAt)
	return &result, nil
}

func decodeRunItemRecord(row deliverydb.DeliveryRunItem, err error) (*RunItemRecord, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, storageError(err)
	}
	if !(ItemKind(row.ItemKind)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	if !(ItemStatus(row.Status)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	var result RunItemRecord
	result.ID = int64(row.ID)
	result.DeliveryRunID = int64(row.DeliveryRunID)
	result.ItemKind = ItemKind(row.ItemKind)
	result.ItemKey = string(row.ItemKey)
	result.UserID = (*int64)(row.UserID)
	result.ArticleID = (*int64)(row.ArticleID)
	result.Status = ItemStatus(row.Status)
	result.LegacyStatus = (*string)(row.LegacyStatus)
	result.OwnerID = (*string)(row.OwnerID)
	result.LeaseExpiresAt = (*float64)(row.LeaseExpiresAt)
	result.AttemptCount = int64(row.AttemptCount)
	result.ResultJSON = (*string)(row.ResultJSON)
	result.ErrorCode = (*string)(row.ErrorCode)
	result.Revision = int64(row.Revision)
	result.CreatedAt = float64(row.CreatedAt)
	result.StartedAt = (*float64)(row.StartedAt)
	result.UpdatedAt = float64(row.UpdatedAt)
	result.FinishedAt = (*float64)(row.FinishedAt)
	return &result, nil
}

func decodeDedupeRecord(row deliverydb.DeliveryDedupe, err error) (*DedupeRecord, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, storageError(err)
	}
	if !(Workflow(row.Workflow)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	if !(DedupeStatus(row.Status)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	var result DedupeRecord
	result.ID = int64(row.ID)
	result.Workflow = Workflow(row.Workflow)
	result.DBName = string(row.DBName)
	result.UserID = int64(row.UserID)
	result.ArticleID = int64(row.ArticleID)
	result.DeliveryRunID = (*int64)(row.DeliveryRunID)
	result.Status = DedupeStatus(row.Status)
	result.MessageID = (*string)(row.MessageID)
	result.ReservationOwner = (*string)(row.ReservationOwner)
	result.LegacyDeliveredAt = (*string)(row.LegacyDeliveredAt)
	result.Revision = int64(row.Revision)
	result.ReservedAt = float64(row.ReservedAt)
	result.DeliveredAt = (*float64)(row.DeliveredAt)
	result.UpdatedAt = float64(row.UpdatedAt)
	return &result, nil
}

func decodeLeaseRecord(row deliverydb.DeliveryLease, err error) (*LeaseRecord, error) {
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, storageError(err)
	}
	if !(Workflow(row.Workflow)).valid() {
		return nil, &Error{Kind: "sqlite"}
	}
	var result LeaseRecord
	result.ID = int64(row.ID)
	result.Workflow = Workflow(row.Workflow)
	result.DBName = string(row.DBName)
	result.DeliveryRunID = (*int64)(row.DeliveryRunID)
	result.OwnerID = (*string)(row.OwnerID)
	result.Revision = int64(row.Revision)
	result.AcquiredAt = (*float64)(row.AcquiredAt)
	result.HeartbeatAt = (*float64)(row.HeartbeatAt)
	result.ExpiresAt = (*float64)(row.ExpiresAt)
	result.UpdatedAt = float64(row.UpdatedAt)
	return &result, nil
}

func scanPreparedItem(scanner *sql.Row) (*RunItemRecord, error) {
	var row deliverydb.DeliveryRunItem
	err := scanner.Scan(&row.ID, &row.DeliveryRunID, &row.ItemKind, &row.ItemKey, &row.UserID, &row.ArticleID, &row.Status, &row.LegacyStatus, &row.OwnerID, &row.LeaseExpiresAt, &row.AttemptCount, &row.ResultJSON, &row.ErrorCode, &row.Revision, &row.CreatedAt, &row.StartedAt, &row.UpdatedAt, &row.FinishedAt)
	return decodeRunItemRecord(row, err)
}

func loadRunRecord(ctx context.Context, connection *sql.Conn, id int64) (*RunRecord, error) {
	return decodeRunRecord(queries.Run(ctx, connection, deliverydb.RunParams{ID: sqlite.Integer(id)}))
}
func loadRunItemRecord(ctx context.Context, connection *sql.Conn, id int64) (*RunItemRecord, error) {
	return decodeRunItemRecord(queries.Item(ctx, connection, deliverydb.ItemParams{ID: sqlite.Integer(id)}))
}
func loadDedupeRecord(ctx context.Context, connection *sql.Conn, id int64) (*DedupeRecord, error) {
	return decodeDedupeRecord(queries.DedupeByID(ctx, connection, deliverydb.DedupeByIDParams{ID: sqlite.Integer(id)}))
}
func loadCheckpointRecord(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string) (*CheckpointRecord, error) {
	return decodeCheckpointRecord(queries.Checkpoint(ctx, connection, deliverydb.CheckpointParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName)}))
}
func loadLeaseRecord(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string) (*LeaseRecord, error) {
	return decodeLeaseRecord(queries.Lease(ctx, connection, deliverydb.LeaseParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName)}))
}
