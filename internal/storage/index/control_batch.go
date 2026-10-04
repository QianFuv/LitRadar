package index

import (
	"context"
	"database/sql"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// BatchJournalState counts durable same-batch coverage separately from unfinished traversals.
type BatchJournalState struct {
	Completed uint64 `json:"completed"`
	InFlight  uint64 `json:"in_flight"`
}

// LegacyBatchAdoption records the coherent epoch adopted from pre-v4 provider control state.
type LegacyBatchAdoption struct {
	StartedAt          *string `json:"started_at"`
	CheckpointsAdopted uint64  `json:"checkpoints_adopted"`
	AnchorsAdopted     uint64  `json:"anchors_adopted"`
}

// HasCatalogAliasSyncState checks all provider namespaces before a maintained alias is adopted.
func HasCatalogAliasSyncState(ctx context.Context, connection *sql.Conn, catalog string, aliases []string) (bool, error) {
	for _, alias := range aliases {
		var exists sqlite.Integer
		if err := connection.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM provider_sync_anchors WHERE catalog_name=?1 AND catalog_id=?2 UNION ALL SELECT 1 FROM provider_run_checkpoints WHERE catalog_name=?1 AND catalog_id=?2)`, catalog, alias).Scan(&exists); err != nil {
			return false, err
		}
		if exists != 0 {
			return true, nil
		}
	}
	return false, nil
}

// ReadBatchJournalState counts completion and in-flight work without extrapolating failures.
func ReadBatchJournalState(ctx context.Context, connection *sql.Conn, catalog, provider, batch string) (BatchJournalState, error) {
	var result BatchJournalState
	if err := validateBatchId(batch); err != nil {
		return result, err
	}
	var completed, inFlight sqlite.Integer
	if err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM provider_sync_anchors WHERE catalog_name=?1 AND provider_name=?2 AND completed_batch_id=?3", catalog, provider, batch).Scan(&completed); err != nil {
		return result, err
	}
	if err := connection.QueryRowContext(ctx, "SELECT COUNT(*) FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND batch_id=?3", catalog, provider, batch).Scan(&inFlight); err != nil {
		return result, err
	}
	if completed < 0 {
		return result, invalidSync("same-batch completed journal count is invalid")
	}
	if inFlight < 0 {
		return result, invalidSync("same-batch in-flight journal count is invalid")
	}
	return BatchJournalState{uint64(completed), uint64(inFlight)}, nil
}

// AdoptLegacyBatchState requires explicit single-catalog selection and an entirely unleased coherent epoch.
func AdoptLegacyBatchState(ctx context.Context, connection *sql.Conn, catalog, provider, batch string, mode domain.IndexSyncMode, canAdopt bool) (LegacyBatchAdoption, error) {
	var outcome LegacyBatchAdoption
	if err := validateBatchId(batch); err != nil {
		return outcome, err
	}
	err := immediate(ctx, connection, func() error {
		rows, err := connection.QueryContext(ctx, "SELECT sync_mode,started_at FROM provider_run_checkpoints WHERE catalog_name=?1 AND provider_name=?2 AND batch_id IS NULL ORDER BY catalog_id", catalog, provider)
		if err != nil {
			return err
		}
		legacy := [][2]sqlite.Text{}
		for rows.Next() {
			var value [2]sqlite.Text
			if err := rows.Scan(&value[0], &value[1]); err != nil {
				rows.Close()
				return err
			}
			legacy = append(legacy, value)
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return err
		}
		if len(legacy) == 0 {
			return nil
		}
		var hasLease sqlite.Integer
		if err := connection.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM provider_leases WHERE catalog_name=?1 AND provider_name=?2)", catalog, provider).Scan(&hasLease); err != nil {
			return err
		}
		if hasLease != 0 {
			return invalidSync("legacy batch adoption requires an unleased catalog")
		}
		if !canAdopt {
			return invalidSync("legacy checkpoint adoption requires explicit single-CSV selection")
		}
		started := string(legacy[0][1])
		for _, value := range legacy {
			if string(value[0]) != string(mode) || string(value[1]) != started {
				return invalidSync("legacy checkpoints do not share one mode and start epoch")
			}
		}
		checkpoints, err := connection.ExecContext(ctx, "UPDATE provider_run_checkpoints SET batch_id=?3 WHERE catalog_name=?1 AND provider_name=?2 AND batch_id IS NULL", catalog, provider, batch)
		if err != nil {
			return err
		}
		anchors, err := connection.ExecContext(ctx, "UPDATE provider_sync_anchors SET completed_batch_id=?3 WHERE catalog_name=?1 AND provider_name=?2 AND completed_batch_id IS NULL AND completed_at=?4", catalog, provider, batch, started)
		if err != nil {
			return err
		}
		checkpointCount, err := checkpoints.RowsAffected()
		if err != nil {
			return err
		}
		anchorCount, err := anchors.RowsAffected()
		if err != nil {
			return err
		}
		outcome = LegacyBatchAdoption{&started, uint64(checkpointCount), uint64(anchorCount)}
		return nil
	})
	if err != nil {
		return LegacyBatchAdoption{}, err
	}
	return outcome, nil
}

// AbandonBatchCheckpoints removes only the abandoned batch's traversal positions, retaining anchors and leases.
func AbandonBatchCheckpoints(ctx context.Context, connection *sql.Conn, batch string) (uint64, error) {
	if err := validateBatchId(batch); err != nil {
		return 0, err
	}
	result, err := connection.ExecContext(ctx, "DELETE FROM provider_run_checkpoints WHERE batch_id=?1", batch)
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	return uint64(count), err
}
