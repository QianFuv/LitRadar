package index

import (
	"context"
	"database/sql"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/controldb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
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
		if err := func() error {
			queryResult, err := controlQueries.HasAliasSyncState(ctx, connection, controldb.HasAliasSyncStateParams{CatalogName: sqlite.Text(catalog), CatalogID: sqlite.Text(alias)})
			if err != nil {
				return err
			}
			exists = sqlite.Integer(dbtypes.Boolean(queryResult))
			return nil
		}(); err != nil {
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
	if err := func() error {
		queryResult, err := controlQueries.BatchAnchorCount(ctx, connection, controldb.BatchAnchorCountParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), CompletedBatchID: new(sqlite.Text(batch))})
		if err != nil {
			return err
		}
		completed = sqlite.Integer(queryResult)
		return nil
	}(); err != nil {
		return result, err
	}
	if err := func() error {
		queryResult, err := controlQueries.BatchRunCount(ctx, connection, controldb.BatchRunCountParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), BatchID: new(sqlite.Text(batch))})
		if err != nil {
			return err
		}
		inFlight = sqlite.Integer(queryResult)
		return nil
	}(); err != nil {
		return result, err
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
		rows, err := controlQueries.LegacyBatchRuns(ctx, connection, controldb.LegacyBatchRunsParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider)})
		if err != nil {
			return err
		}
		legacy := make([][2]sqlite.Text, 0, len(rows))
		for _, row := range rows {
			legacy = append(legacy, [2]sqlite.Text{row.SyncMode, row.StartedAt})
		}
		if len(legacy) == 0 {
			return nil
		}
		var hasLease sqlite.Integer
		if err := func() error {
			queryResult, err := controlQueries.HasProviderLease(ctx, connection, controldb.HasProviderLeaseParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider)})
			if err != nil {
				return err
			}
			hasLease = sqlite.Integer(dbtypes.Boolean(queryResult))
			return nil
		}(); err != nil {
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
		checkpoints, err := controlQueries.AdoptRunBatch(ctx, connection, controldb.AdoptRunBatchParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), BatchID: new(sqlite.Text(batch))})
		if err != nil {
			return err
		}
		anchors, err := controlQueries.AdoptAnchorBatch(ctx, connection, controldb.AdoptAnchorBatchParams{CatalogName: sqlite.Text(catalog), ProviderName: sqlite.Text(provider), CompletedBatchID: new(sqlite.Text(batch)), CompletedAt: sqlite.Text(started)})
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
	result, err := controlQueries.AbandonBatchCheckpoints(ctx, connection, controldb.AbandonBatchCheckpointsParams{BatchID: new(sqlite.Text(batch))})
	if err != nil {
		return 0, err
	}
	count, err := result.RowsAffected()
	return uint64(count), err
}
