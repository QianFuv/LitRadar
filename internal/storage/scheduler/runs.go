package scheduler

import (
	"context"
	"database/sql"
	"math"

	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// Claim identifies the owner and latest task definition for a durable run.
type Claim struct {
	RunId        int64       `json:"run_id"`
	ScheduledFor int64       `json:"scheduled_for"`
	WorkerId     string      `json:"worker_id"`
	Task         domain.Task `json:"task"`
}

// Admission separates absence and contention from a successfully committed manual claim.
type Admission struct {
	Status string
	Claim  *Claim
}

// ClaimManual allows disabled typed tasks while serializing with every active task execution.
func (repository *Repository) ClaimManual(ctx context.Context, id int64, worker string, now, lease float64) (Admission, error) {
	result := Admission{Status: "not_found"}
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		task, err := getTask(ctx, connection, id)
		if err != nil || task == nil {
			return err
		}
		if task.Job == nil {
			return &domain.ValidationError{Message: "Legacy task requires a typed job"}
		}
		if err := task.Job.Validate(); err != nil {
			return err
		}
		if err := domain.ValidateTiming(task.Timezone, task.TimeoutSeconds); err != nil {
			return err
		}
		if err := reconcile(ctx, connection, now); err != nil {
			return err
		}
		var active sqlite.Integer
		if err := connection.QueryRowContext(ctx, `SELECT EXISTS(SELECT 1 FROM scheduled_task_runs WHERE task_id=? AND status IN ('claimed','running'))`, id).Scan(&active); err != nil {
			return err
		}
		if active != 0 {
			result.Status = "busy"
			return nil
		}
		scheduledFor := saturatingInteger(math.Floor(now))
		inserted, err := connection.ExecContext(ctx, `INSERT INTO scheduled_task_runs(task_id,task_name,scheduled_for,trigger_kind,status,worker_id,claimed_at,claim_expires_at) VALUES(?,?,?,'manual','claimed',?,?,?)`, id, task.Name, scheduledFor, worker, now, now+lease)
		if err != nil {
			return err
		}
		runId, err := inserted.LastInsertId()
		if err != nil {
			return err
		}
		result = Admission{Status: "claimed", Claim: &Claim{runId, scheduledFor, worker, *task}}
		return nil
	})
	return result, err
}

func saturatingInteger(value float64) int64 {
	if math.IsNaN(value) {
		return 0
	}
	if value >= math.MaxInt64 {
		return math.MaxInt64
	}
	if value <= math.MinInt64 {
		return math.MinInt64
	}
	return int64(value)
}

// Enqueue retains scheduled-slot identity and excludes manual history from the coalescing watermark.
func (repository *Repository) Enqueue(ctx context.Context, task domain.Task, slots []int64) (int, error) {
	if len(slots) == 0 {
		return 0, nil
	}
	inserted := 0
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		selected := slots
		if task.Coalesce {
			latest := slots[len(slots)-1]
			var known sqlite.OptionalInteger
			if err := connection.QueryRowContext(ctx, `SELECT MAX(scheduled_for) FROM scheduled_task_runs WHERE task_id=? AND trigger_kind='scheduled'`, task.Id).Scan(&known); err != nil {
				return err
			}
			choice := latest
			if known.Value != nil {
				choice = max(choice, *known.Value)
			}
			if _, err := connection.ExecContext(ctx, `DELETE FROM scheduled_task_runs WHERE task_id=? AND trigger_kind='scheduled' AND status='pending' AND scheduled_for<?`, task.Id, choice); err != nil {
				return err
			}
			if choice != latest {
				return nil
			}
			selected = []int64{choice}
		}
		for _, slot := range selected {
			result, err := connection.ExecContext(ctx, `INSERT OR IGNORE INTO scheduled_task_runs(task_id,task_name,scheduled_for,status) VALUES(?,?,?,'pending')`, task.Id, task.Name, slot)
			if err != nil {
				return err
			}
			count, err := result.RowsAffected()
			if err != nil {
				return err
			}
			inserted += int(count)
		}
		return nil
	})
	return inserted, err
}

// ClaimReady reconciles stale work and leases only as many tasks as can start immediately.
func (repository *Repository) ClaimReady(ctx context.Context, worker string, now, lease float64, capacity uint64) ([]Claim, error) {
	claims := []Claim{}
	if capacity == 0 {
		return claims, nil
	}
	type candidate struct{ runId, taskId, slot int64 }
	candidates := []candidate{}
	claimed := []candidate{}
	limit := int64(min(capacity, uint64(math.MaxInt64)))
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := reconcile(ctx, connection, now); err != nil {
			return err
		}
		rows, err := connection.QueryContext(ctx, `SELECT run.id,run.task_id,run.scheduled_for FROM scheduled_task_runs AS run JOIN scheduled_tasks AS task ON task.id=run.task_id
 WHERE run.status='pending' AND run.trigger_kind='scheduled' AND task.enabled=1 AND task.job_spec IS NOT NULL
 AND NOT EXISTS(SELECT 1 FROM scheduled_task_runs AS active WHERE active.task_id=run.task_id AND active.status IN ('claimed','running'))
 AND NOT EXISTS(SELECT 1 FROM scheduled_task_runs AS earlier WHERE earlier.task_id=run.task_id AND earlier.status='pending' AND earlier.trigger_kind='scheduled' AND (earlier.scheduled_for<run.scheduled_for OR (earlier.scheduled_for=run.scheduled_for AND earlier.id<run.id)))
 ORDER BY run.scheduled_for,run.id LIMIT ?`, limit)
		if err != nil {
			return err
		}
		for rows.Next() {
			var runId, taskId, slot sqlite.Integer
			if err := rows.Scan(&runId, &taskId, &slot); err != nil {
				rows.Close()
				return err
			}
			candidates = append(candidates, candidate{int64(runId), int64(taskId), int64(slot)})
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return err
		}
		for _, item := range candidates {
			didClaim, err := execute(ctx, connection, `UPDATE scheduled_task_runs SET status='claimed',worker_id=?,claimed_at=?,claim_expires_at=? WHERE id=? AND status='pending' AND NOT EXISTS(SELECT 1 FROM scheduled_task_runs AS active WHERE active.task_id=? AND active.status IN ('claimed','running'))`, worker, now, now+lease, item.runId, item.taskId)
			if err != nil {
				return err
			}
			if didClaim {
				claimed = append(claimed, item)
			}
		}
		return nil
	})
	if err != nil {
		return nil, err
	}
	for _, item := range claimed {
		task, err := repository.Get(ctx, item.taskId)
		if err != nil {
			return nil, err
		}
		if task == nil || !task.Enabled || task.Job == nil {
			err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
				_, err := connection.ExecContext(ctx, `UPDATE scheduled_task_runs SET status='error',finished_at=?,claim_expires_at=NULL,output_summary='Task is no longer executable' WHERE id=? AND worker_id=? AND status='claimed'`, now, item.runId, worker)
				return err
			})
			if err != nil {
				return nil, err
			}
			continue
		}
		claims = append(claims, Claim{item.runId, item.slot, worker, *task})
	}
	return claims, nil
}

func reconcile(ctx context.Context, connection *sql.Conn, now float64) error {
	statements := []string{
		`UPDATE scheduled_task_runs SET status='unknown',finished_at=?1,claim_expires_at=NULL WHERE status='running' AND claim_expires_at<=?1`,
		`UPDATE scheduled_tasks SET last_run_at=?1,last_status='unknown',updated_at=?1 WHERE id IN (SELECT task_id FROM scheduled_task_runs WHERE status='unknown' AND finished_at=?1)`,
		`UPDATE scheduled_task_runs SET status='pending',worker_id=NULL,claim_expires_at=NULL,claimed_at=NULL WHERE status='claimed' AND trigger_kind='scheduled' AND claim_expires_at<=?1`,
		`UPDATE scheduled_task_runs SET status='cancelled',finished_at=?1,claim_expires_at=NULL WHERE status='claimed' AND trigger_kind='manual' AND claim_expires_at<=?1`,
		`UPDATE scheduled_tasks SET last_run_at=?1,last_status='cancelled',updated_at=?1 WHERE id IN (SELECT task_id FROM scheduled_task_runs WHERE trigger_kind='manual' AND status='cancelled' AND finished_at=?1)`,
	}
	for _, query := range statements {
		if _, err := connection.ExecContext(ctx, query, now); err != nil {
			return err
		}
	}
	_, err := connection.ExecContext(ctx, `DELETE FROM scheduled_task_runs WHERE status='pending' AND trigger_kind='scheduled' AND task_id IN (SELECT id FROM scheduled_tasks WHERE coalesce=1) AND scheduled_for<(SELECT MAX(latest.scheduled_for) FROM scheduled_task_runs AS latest WHERE latest.task_id=scheduled_task_runs.task_id AND latest.trigger_kind='scheduled')`)
	return err
}

// StartRun fences owner and claimed state; expiration takes effect only through reconciliation.
func (repository *Repository) StartRun(ctx context.Context, id int64, worker string, now, lease float64) (bool, error) {
	var didStart bool
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		didStart, err = execute(ctx, connection, `UPDATE scheduled_task_runs SET status='running',started_at=?,claim_expires_at=? WHERE id=? AND worker_id=? AND status='claimed'`, now, now+lease, id, worker)
		return err
	})
	return didStart, err
}

// HeartbeatRun commits worker liveness even when the requested running claim is no longer owned.
func (repository *Repository) HeartbeatRun(ctx context.Context, id int64, worker string, now, lease float64) (bool, error) {
	var didRenew bool
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := heartbeat(ctx, connection, worker, now); err != nil {
			return err
		}
		var err error
		didRenew, err = execute(ctx, connection, `UPDATE scheduled_task_runs SET claim_expires_at=? WHERE id=? AND worker_id=? AND status='running'`, now+lease, id, worker)
		return err
	})
	return didRenew, err
}

// FinishRun atomically records a terminal result and the task summary, including a deleted task's retained run.
func (repository *Repository) FinishRun(ctx context.Context, claim Claim, status domain.State, summary string, now float64) (bool, error) {
	if !status.IsTerminal() {
		return false, &domain.ValidationError{Message: "Scheduled run result must be terminal"}
	}
	var didFinish bool
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		var err error
		didFinish, err = execute(ctx, connection, `UPDATE scheduled_task_runs SET status=?,finished_at=?,claim_expires_at=NULL,output_summary=? WHERE id=? AND worker_id=? AND status IN ('claimed','running')`, status, now, summary, claim.RunId, claim.WorkerId)
		if err != nil {
			return err
		}
		if didFinish {
			_, err = connection.ExecContext(ctx, `UPDATE scheduled_tasks SET last_run_at=?1,last_status=?2,updated_at=?1 WHERE id=?3`, now, status, claim.Task.Id)
		}
		return err
	})
	return didFinish, err
}
