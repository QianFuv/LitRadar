package delivery

import (
	"context"
	"database/sql"
)

// LoadCheckpoint returns the existing workflow checkpoint without admitting work.
func (repository *Repository) LoadCheckpoint(ctx context.Context, workflow Workflow, dbName string) (*CheckpointRecord, error) {
	if err := validateDbName(dbName); err != nil {
		return nil, err
	}
	return scanCheckpointRecord(repository.database.QueryRowContext(ctx, "SELECT "+checkpointColumns+" FROM delivery_checkpoints WHERE workflow=? AND db_name=?", workflow, dbName))
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
		err = executeCas(ctx, connection, `INSERT INTO delivery_checkpoints(workflow,db_name,status,legacy_status,snapshot_json,last_completed_run_at,revision,legacy_source_hash,legacy_source_name,legacy_imported_at,created_at,updated_at) VALUES(?,?,?,NULL,?,?,0,NULL,NULL,NULL,?,?) ON CONFLICT(workflow,db_name) DO NOTHING`, workflow, dbName, update.Status, update.SnapshotJson, update.LastCompletedRunAt, update.UpdatedAt, update.UpdatedAt)
	} else {
		err = executeCas(ctx, connection, `UPDATE delivery_checkpoints SET status=?,legacy_status=NULL,snapshot_json=?,last_completed_run_at=?,revision=revision+1,updated_at=? WHERE workflow=? AND db_name=? AND revision=?`, update.Status, update.SnapshotJson, update.LastCompletedRunAt, update.UpdatedAt, workflow, dbName, *expected)
	}
	return err
}
