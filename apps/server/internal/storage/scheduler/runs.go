package scheduler

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"math"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/schedulerdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// Claim identifies the owner and latest task definition for a durable run.
type Claim struct {
	RunID        int64       `json:"run_id"`
	ScheduledFor int64       `json:"scheduled_for"`
	WorkerID     string      `json:"worker_id"`
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
		if err := func() error {
			queryResult, err := queries.HasActiveRun(ctx, connection, schedulerdb.HasActiveRunParams{TaskID: sqlite.Integer(id)})
			if err != nil {
				return err
			}
			active = sqlite.Integer(dbtypes.Boolean(queryResult))
			return nil
		}(); err != nil {
			return err
		}
		if active != 0 {
			result.Status = "busy"
			return nil
		}
		scheduledFor := saturatingInteger(math.Floor(now))
		inserted, err := queries.CreateManualRun(ctx, connection, schedulerdb.CreateManualRunParams{TaskID: sqlite.Integer(id), TaskName: sqlite.Text(task.Name), ScheduledFor: sqlite.Integer(scheduledFor), WorkerID: new(sqlite.Text(worker)), ClaimedAt: new(sqlite.Number(now)), ClaimExpiresAt: new(sqlite.Number(now + lease))})
		if err != nil {
			return err
		}
		runID, err := inserted.LastInsertId()
		if err != nil {
			return err
		}
		result = Admission{Status: "claimed", Claim: &Claim{runID, scheduledFor, worker, *task}}
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
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) (resultError error) {
		selected := slots
		if task.Coalesce {
			latest := slots[len(slots)-1]
			var known sqlite.OptionalInteger
			if err := func() error {
				queryResult, err := queries.LatestScheduledSlot(ctx, connection, schedulerdb.LatestScheduledSlotParams{TaskID: sqlite.Integer(task.ID)})
				if err != nil {
					return err
				}
				if err := (&known).Scan(queryResult); err != nil {
					return err
				}
				return nil
			}(); err != nil {
				return err
			}
			choice := latest
			if known.Value != nil {
				choice = max(choice, *known.Value)
			}
			if _, err := queries.PrunePendingRuns(ctx, connection, schedulerdb.PrunePendingRunsParams{TaskID: sqlite.Integer(task.ID), ScheduledFor: sqlite.Integer(choice)}); err != nil {
				return err
			}
			if choice != latest {
				return nil
			}
			selected = []int64{choice}
		}
		var statement *sql.Stmt
		if len(selected) > 1 {
			var err error
			statement, err = connection.PrepareContext(ctx, schedulerdb.EnqueueRun)
			if err != nil {
				return err
			}
			defer func() {
				if closeError := statement.Close(); closeError != nil {
					resultError = errors.Join(resultError, fmt.Errorf("close statement: %w", closeError))
				}
			}()
		}
		for _, slot := range selected {
			var result sql.Result
			var err error
			if statement == nil {
				result, err = queries.EnqueueRun(ctx, connection, schedulerdb.EnqueueRunParams{TaskID: sqlite.Integer(task.ID), TaskName: sqlite.Text(task.Name), ScheduledFor: sqlite.Integer(slot)})
			} else {
				result, err = statement.ExecContext(ctx, task.ID, task.Name, slot)
			}
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
	type candidate struct{ runID, taskID, slot int64 }
	candidates := []candidate{}
	claimed := []candidate{}
	limit := int64(min(capacity, uint64(math.MaxInt64)))
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := reconcile(ctx, connection, now); err != nil {
			return err
		}
		rows, err := queries.ReadyRuns(ctx, connection, schedulerdb.ReadyRunsParams{Limit: limit})
		if err != nil {
			return err
		}
		for _, row := range rows {
			candidates = append(candidates, candidate{int64(row.ID), int64(row.TaskID), int64(row.ScheduledFor)})
		}
		for _, item := range candidates {
			didClaim, err := execute(queries.ClaimRun(ctx, connection, schedulerdb.ClaimRunParams{WorkerID: new(sqlite.Text(worker)), ClaimedAt: new(sqlite.Number(now)), ClaimExpiresAt: new(sqlite.Number(now + lease)), ID: sqlite.Integer(item.runID), TaskID: sqlite.Integer(item.taskID)}))
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
		task, err := repository.Get(ctx, item.taskID)
		if err != nil {
			return nil, err
		}
		if task == nil || !task.Enabled || task.Job == nil {
			err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
				_, err := queries.CancelUnexecutableRun(ctx, connection, schedulerdb.CancelUnexecutableRunParams{FinishedAt: new(sqlite.Number(now)), ID: sqlite.Integer(item.runID), WorkerID: new(sqlite.Text(worker))})
				return err
			})
			if err != nil {
				return nil, err
			}
			continue
		}
		claims = append(claims, Claim{item.runID, item.slot, worker, *task})
	}
	return claims, nil
}

func reconcile(ctx context.Context, connection *sql.Conn, now float64) error {
	if _, err := queries.ExpireRunningRuns(ctx, connection, schedulerdb.ExpireRunningRunsParams{FinishedAt: new(sqlite.Number(now))}); err != nil {
		return err
	}
	if _, err := queries.RecordUnknownTasks(ctx, connection, schedulerdb.RecordUnknownTasksParams{LastRunAt: new(sqlite.Number(now))}); err != nil {
		return err
	}
	if _, err := queries.ReleaseScheduledClaims(ctx, connection, schedulerdb.ReleaseScheduledClaimsParams{ClaimExpiresAt: new(sqlite.Number(now))}); err != nil {
		return err
	}
	if _, err := queries.CancelManualClaims(ctx, connection, schedulerdb.CancelManualClaimsParams{FinishedAt: new(sqlite.Number(now))}); err != nil {
		return err
	}
	if _, err := queries.RecordCancelledTasks(ctx, connection, schedulerdb.RecordCancelledTasksParams{LastRunAt: new(sqlite.Number(now))}); err != nil {
		return err
	}
	_, err := queries.PruneCoalescedRuns(ctx, connection)
	return err
}

// StartRun fences owner and claimed state; expiration takes effect only through reconciliation.
func (repository *Repository) StartRun(ctx context.Context, id int64, worker string, now, lease float64) (bool, error) {
	var didStart bool
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		didStart, err = execute(queries.StartRun(ctx, connection, schedulerdb.StartRunParams{StartedAt: new(sqlite.Number(now)), ClaimExpiresAt: new(sqlite.Number(now + lease)), ID: sqlite.Integer(id), WorkerID: new(sqlite.Text(worker))}))
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
		didRenew, err = execute(queries.RenewRun(ctx, connection, schedulerdb.RenewRunParams{ClaimExpiresAt: new(sqlite.Number(now + lease)), ID: sqlite.Integer(id), WorkerID: new(sqlite.Text(worker))}))
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
		didFinish, err = execute(queries.FinishRun(ctx, connection, schedulerdb.FinishRunParams{Status: sqlite.Text(status), FinishedAt: new(sqlite.Number(now)), OutputSummary: sqlite.Text(summary), ID: sqlite.Integer(claim.RunID), WorkerID: new(sqlite.Text(claim.WorkerID))}))
		if err != nil {
			return err
		}
		if didFinish {
			_, err = queries.RecordTaskResult(ctx, connection, schedulerdb.RecordTaskResultParams{LastRunAt: new(sqlite.Number(now)), LastStatus: sqlite.Text(status), ID: sqlite.Integer(claim.Task.ID)})
		}
		return err
	})
	return didFinish, err
}
