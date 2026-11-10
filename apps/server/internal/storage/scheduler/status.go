package scheduler

import (
	"context"
	"database/sql"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/schedulerdb"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// LastCheckedAt reports the cursor and fails if its required singleton row is missing.
func (repository *Repository) LastCheckedAt(ctx context.Context) (*float64, error) {
	var value *sqlite.Number
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		value, err = queries.Cursor(ctx, connection)
		return err
	})
	return (*float64)(value), err
}

// RecordCheck never moves the persisted cursor backward, including when the clock regresses.
func (repository *Repository) RecordCheck(ctx context.Context, now float64) error {
	return repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := queries.RecordCheck(ctx, connection, schedulerdb.RecordCheckParams{LastCheckedAt: new(sqlite.Number(now))})
		return err
	})
}

func heartbeat(ctx context.Context, connection *sql.Conn, worker string, now float64) error {
	if _, err := queries.WorkerHeartbeat(ctx, connection, schedulerdb.WorkerHeartbeatParams{WorkerID: sqlite.Text(worker), StartedAt: sqlite.Number(now)}); err != nil {
		return err
	}
	_, err := queries.ServiceHeartbeat(ctx, connection, schedulerdb.ServiceHeartbeatParams{InstanceID: sqlite.Text(worker), StartedAt: sqlite.Number(now)})
	return err
}

// RecordHeartbeat preserves the original nontransactional heartbeat writes and seven-day worker pruning.
func (repository *Repository) RecordHeartbeat(ctx context.Context, worker string, now float64) error {
	return repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		if err := heartbeat(ctx, connection, worker, now); err != nil {
			return err
		}
		_, err := queries.PruneWorkers(ctx, connection, schedulerdb.PruneWorkersParams{WorkerID: sqlite.Text(worker), HeartbeatAt: sqlite.Number(now - 604800)})
		return err
	})
}

// Status returns current heartbeat health and history in the original administrator ordering.
func (repository *Repository) Status(ctx context.Context, now, healthyWindow float64, limit uint64) (domain.Status, error) {
	result := domain.Status{Workers: []domain.Worker{}, RecentRuns: []domain.Run{}}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		cursor, err := queries.Cursor(ctx, connection)
		if err != nil {
			return err
		}
		result.LastCheckedAt = (*float64)(cursor)
		workers, err := queries.Workers(ctx, connection)
		if err != nil {
			return err
		}
		for _, row := range workers {
			result.Workers = append(result.Workers, domain.Worker{WorkerId: string(row.WorkerID), StartedAt: float64(row.StartedAt), HeartbeatAt: float64(row.HeartbeatAt), IsHealthy: float64(row.HeartbeatAt) >= now-healthyWindow})
		}
		runs, err := queries.RecentRuns(ctx, connection, schedulerdb.RecentRunsParams{Limit: int64(limit)})
		if err != nil {
			return err
		}
		for _, row := range runs {
			result.RecentRuns = append(result.RecentRuns, domain.Run{Id: int64(row.ID), TaskId: int64(row.TaskID), TaskName: string(row.TaskName), ScheduledFor: int64(row.ScheduledFor), Status: domain.PersistedState(string(row.Status)), WorkerId: (*string)(row.WorkerID), ClaimedAt: (*float64)(row.ClaimedAt), StartedAt: (*float64)(row.StartedAt), FinishedAt: (*float64)(row.FinishedAt)})
		}
		return nil
	})
	return result, err
}
