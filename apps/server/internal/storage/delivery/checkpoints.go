package delivery

import (
	"context"
	"database/sql"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/deliverydb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// LoadCheckpoint returns the existing workflow checkpoint without admitting work.
func (repository *Repository) LoadCheckpoint(ctx context.Context, workflow Workflow, dbName string) (*CheckpointRecord, error) {
	if err := validateDbName(dbName); err != nil {
		return nil, err
	}
	return decodeCheckpointRecord(queries.Checkpoint(ctx, repository.database, deliverydb.CheckpointParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName)}))
}

// CompareAndSwapCheckpoint creates the first snapshot or replaces exactly its observed revision.
func (repository *Repository) CompareAndSwapCheckpoint(ctx context.Context, workflow Workflow, dbName string, expected *int64, update CheckpointUpdate) (*CheckpointRecord, error) {
	if err := validateDbName(dbName); err != nil {
		return nil, err
	}
	if err := validateJson(update.SnapshotJson); err != nil {
		return nil, err
	}
	if err := validateTime(update.UpdatedAt, "Checkpoint update time is invalid"); err != nil {
		return nil, err
	}
	if expected != nil && *expected < 0 {
		return nil, invalid("Checkpoint revision is invalid")
	}
	var record *CheckpointRecord
	err := repository.immediate(ctx, func(connection *sql.Conn) error {
		var err error
		record, err = checkpointCas(ctx, connection, workflow, dbName, expected, update)
		return err
	})
	return record, err
}

func checkpointCas(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string, expected *int64, update CheckpointUpdate) (*CheckpointRecord, error) {
	if err := writeCheckpoint(ctx, connection, workflow, dbName, expected, update); err != nil {
		return nil, err
	}
	return requireRecord(loadCheckpointRecord(ctx, connection, workflow, dbName))
}

func writeCheckpoint(ctx context.Context, connection *sql.Conn, workflow Workflow, dbName string, expected *int64, update CheckpointUpdate) error {
	if !workflow.valid() || !update.Status.valid() {
		return invalid("Delivery checkpoint classification is invalid")
	}
	var err error
	if expected == nil {
		err = checkCAS(queries.CreateCheckpoint(ctx, connection, deliverydb.CreateCheckpointParams{Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), Status: sqlite.Text(update.Status), SnapshotJSON: sqlite.Text(update.SnapshotJson), LastCompletedRunAt: (*sqlite.Text)(update.LastCompletedRunAt), CreatedAt: sqlite.Number(update.UpdatedAt), UpdatedAt: sqlite.Number(update.UpdatedAt)}))
	} else {
		err = checkCAS(queries.UpdateCheckpoint(ctx, connection, deliverydb.UpdateCheckpointParams{Status: sqlite.Text(update.Status), SnapshotJSON: sqlite.Text(update.SnapshotJson), LastCompletedRunAt: (*sqlite.Text)(update.LastCompletedRunAt), UpdatedAt: sqlite.Number(update.UpdatedAt), Workflow: sqlite.Text(workflow), DBName: sqlite.Text(dbName), Revision: sqlite.Integer(*expected)}))
	}
	return err
}
