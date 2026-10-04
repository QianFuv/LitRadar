package scheduler

import (
	"context"
	"database/sql"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// LastCheckedAt reports the cursor and fails if its required singleton row is missing.
func (repository *Repository) LastCheckedAt(ctx context.Context) (*float64, error) {
	var value optionalNumber
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		return connection.QueryRowContext(ctx, "SELECT last_checked_at FROM scheduler_state WHERE id=1").Scan(&value)
	})
	return value.Value, err
}

// RecordCheck never moves the persisted cursor backward, including when the clock regresses.
func (repository *Repository) RecordCheck(ctx context.Context, now float64) error {
	return repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, `UPDATE scheduler_state SET last_checked_at=CASE WHEN last_checked_at IS NULL OR last_checked_at<?1 THEN ?1 ELSE last_checked_at END WHERE id=1`, now)
		return err
	})
}

func heartbeat(ctx context.Context, connection *sql.Conn, worker string, now float64) error {
	if _, err := connection.ExecContext(ctx, `INSERT INTO scheduler_workers(worker_id,started_at,heartbeat_at) VALUES(?1,?2,?2) ON CONFLICT(worker_id) DO UPDATE SET heartbeat_at=excluded.heartbeat_at`, worker, now); err != nil {
		return err
	}
	_, err := connection.ExecContext(ctx, `INSERT INTO service_heartbeats(service,instance_id,started_at,heartbeat_at) VALUES('worker',?1,?2,?2) ON CONFLICT(service,instance_id) DO UPDATE SET heartbeat_at=excluded.heartbeat_at`, worker, now)
	return err
}

// RecordHeartbeat preserves the original nontransactional heartbeat writes and seven-day worker pruning.
func (repository *Repository) RecordHeartbeat(ctx context.Context, worker string, now float64) error {
	return repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		if err := heartbeat(ctx, connection, worker, now); err != nil {
			return err
		}
		_, err := connection.ExecContext(ctx, "DELETE FROM scheduler_workers WHERE worker_id<>? AND heartbeat_at<?", worker, now-604800)
		return err
	})
}

// Status returns current heartbeat health and history in the original administrator ordering.
func (repository *Repository) Status(ctx context.Context, now, healthyWindow float64, limit uint64) (domain.Status, error) {
	result := domain.Status{Workers: []domain.Worker{}, RecentRuns: []domain.Run{}}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var cursor optionalNumber
		if err := connection.QueryRowContext(ctx, "SELECT last_checked_at FROM scheduler_state WHERE id=1").Scan(&cursor); err != nil {
			return err
		}
		result.LastCheckedAt = cursor.Value
		rows, err := connection.QueryContext(ctx, "SELECT worker_id,started_at,heartbeat_at FROM scheduler_workers ORDER BY heartbeat_at DESC")
		if err != nil {
			return err
		}
		for rows.Next() {
			var worker sqlite.Text
			var started, heartbeat sqlite.Number
			if err := rows.Scan(&worker, &started, &heartbeat); err != nil {
				rows.Close()
				return err
			}
			result.Workers = append(result.Workers, domain.Worker{WorkerId: string(worker), StartedAt: float64(started), HeartbeatAt: float64(heartbeat), IsHealthy: float64(heartbeat) >= now-healthyWindow})
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return err
		}
		rows, err = connection.QueryContext(ctx, `SELECT id,task_id,task_name,scheduled_for,status,worker_id,claimed_at,started_at,finished_at FROM scheduled_task_runs ORDER BY scheduled_for DESC,id DESC LIMIT ?`, int64(limit))
		if err != nil {
			return err
		}
		defer rows.Close()
		for rows.Next() {
			var id, taskId, slot sqlite.Integer
			var name, status sqlite.Text
			var worker sqlite.OptionalText
			var claimed, started, finished optionalNumber
			if err := rows.Scan(&id, &taskId, &name, &slot, &status, &worker, &claimed, &started, &finished); err != nil {
				return err
			}
			result.RecentRuns = append(result.RecentRuns, domain.Run{Id: int64(id), TaskId: int64(taskId), TaskName: string(name), ScheduledFor: int64(slot), Status: domain.PersistedState(string(status)), WorkerId: worker.Value, ClaimedAt: claimed.Value, StartedAt: started.Value, FinishedAt: finished.Value})
		}
		return rows.Err()
	})
	return result, err
}
