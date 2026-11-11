// Package scheduler persists task definitions and durable scheduled/manual claims.
package scheduler

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	auditdomain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/schedulerdb"
	dbtypes "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/types"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// Repository shares the existing auth connection policy without implicitly migrating schemas.
type Repository struct{ auth *auth.Repository }

// Open opens a scheduler repository after the caller's explicit auth migration.
func Open(path string) (*Repository, error) {
	repository, err := auth.Open(path)
	if err != nil {
		return nil, err
	}
	return &Repository{repository}, nil
}

// Close releases the underlying connection pool.
func (repository *Repository) Close() error { return repository.auth.Close() }

func nowSeconds() float64 { return float64(time.Now().UnixNano()) / 1e9 }
func authorize(ctx context.Context, connection *sql.Conn, actor *identity.ID) error {
	if actor == nil {
		return nil
	}
	return auth.RequireAdministrator(ctx, connection, *actor)
}
func auditTarget(ctx context.Context, connection *sql.Conn, event *auditdomain.AuditEvent, id int64) error {
	if event == nil {
		return nil
	}
	copy := *event
	copy.TargetID = &id
	return auth.InsertAudit(ctx, connection, &copy)
}

var queries schedulerdb.Queries

func decodeTask(row schedulerdb.TaskRow) (*domain.Task, error) {
	var job *domain.Job
	if row.JobSpec != nil {
		job = new(domain.Job)
		if err := json.Unmarshal([]byte(string(*row.JobSpec)), job); err != nil {
			return nil, err
		}
	}
	if row.TimeoutSeconds < 0 {
		return nil, errors.New("invalid SQLite unsigned integer column")
	}
	return &domain.Task{ID: int64(row.ID), Name: string(row.Name), Job: job, LegacyCommand: (*string)(row.LegacyCommand), Cron: string(row.Cron), Timezone: string(row.Timezone), TimeoutSeconds: uint64(row.TimeoutSeconds), Coalesce: row.Coalesce != 0, Enabled: row.Enabled != 0, LastRunAt: (*float64)(row.LastRunAt), LastStatus: domain.PersistedState(string(row.LastStatus)), CreatedAt: float64(row.CreatedAt), UpdatedAt: float64(row.UpdatedAt)}, nil
}

func getTask(ctx context.Context, connection *sql.Conn, id int64) (*domain.Task, error) {
	row, err := queries.Task(ctx, connection, schedulerdb.TaskParams{ID: sqlite.Integer(id)})
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return decodeTask(row)
}

// Get returns an existing task, including disabled and legacy definitions.
func (repository *Repository) Get(ctx context.Context, id int64) (*domain.Task, error) {
	var task *domain.Task
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error { var err error; task, err = getTask(ctx, connection, id); return err })
	return task, err
}

// List preserves creation-time descending ordering without adding a tie-breaker.
func (repository *Repository) List(ctx context.Context) ([]domain.Task, error) {
	tasks := []domain.Task{}
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		rows, err := queries.Tasks(ctx, connection)
		if err != nil {
			return err
		}
		for _, row := range rows {
			task, err := decodeTask(schedulerdb.TaskRow(row))
			if err != nil {
				return err
			}
			tasks = append(tasks, *task)
		}
		return nil
	})
	return tasks, err
}

// Create validates typed arguments before atomically checking authority, inserting and auditing.
func (repository *Repository) Create(ctx context.Context, input domain.Create, actor *identity.ID, audit *auditdomain.AuditEvent) (*domain.Task, error) {
	if err := input.Job.Validate(); err != nil {
		return nil, err
	}
	if err := domain.ValidateTiming(input.Timezone, input.TimeoutSeconds); err != nil {
		return nil, err
	}
	var task *domain.Task
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		encoded, err := input.Job.MarshalJSON()
		if err != nil {
			return err
		}
		now := nowSeconds()
		result, err := queries.CreateTask(ctx, connection, schedulerdb.CreateTaskParams{Name: sqlite.Text(input.Name), JobSpec: new(sqlite.Text(string(encoded))), Cron: sqlite.Text(input.Cron), Timezone: sqlite.Text(input.Timezone), TimeoutSeconds: sqlite.Integer(input.TimeoutSeconds), Coalesce: sqlite.Integer(dbtypes.Boolean(input.Coalesce)), Enabled: sqlite.Integer(dbtypes.Boolean(input.Enabled)), CreatedAt: sqlite.Number(now), UpdatedAt: sqlite.Number(now)})
		if err != nil {
			return err
		}
		id, err := result.LastInsertId()
		if err != nil {
			return err
		}
		task, err = getTask(ctx, connection, id)
		if err != nil {
			return err
		}
		if task == nil {
			return sql.ErrNoRows
		}
		return auditTarget(ctx, connection, audit, id)
	})
	return task, err
}

// Update merges fields within the write transaction and clears legacy text only for a replacement job.
func (repository *Repository) Update(ctx context.Context, input domain.Update, actor *identity.ID, audit *auditdomain.AuditEvent) (*domain.Task, error) {
	var task *domain.Task
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		current, err := getTask(ctx, connection, input.TaskID)
		if err != nil || current == nil {
			return err
		}
		if input.Job != nil {
			current.Job = input.Job
			current.LegacyCommand = nil
		}
		if current.Job != nil {
			if err := current.Job.Validate(); err != nil {
				return err
			}
		}
		if input.Timezone != nil {
			current.Timezone = *input.Timezone
		}
		if input.TimeoutSeconds != nil {
			current.TimeoutSeconds = *input.TimeoutSeconds
		}
		if err := domain.ValidateTiming(current.Timezone, current.TimeoutSeconds); err != nil {
			return err
		}
		if input.Enabled != nil {
			current.Enabled = *input.Enabled
		}
		if current.Job == nil && current.Enabled {
			return &domain.ValidationError{Message: "A legacy scheduled task must be replaced with a typed job before it can be enabled"}
		}
		if input.Name != nil {
			current.Name = *input.Name
		}
		if input.Cron != nil {
			current.Cron = *input.Cron
		}
		if input.Coalesce != nil {
			current.Coalesce = *input.Coalesce
		}
		var encoded *sqlite.Text
		if current.Job != nil {
			value, err := current.Job.MarshalJSON()
			if err != nil {
				return err
			}
			encoded = new(sqlite.Text(string(value)))
		}
		if _, err := queries.UpdateTask(ctx, connection, schedulerdb.UpdateTaskParams{Name: sqlite.Text(current.Name), JobSpec: encoded, LegacyCommand: (*sqlite.Text)(current.LegacyCommand), Cron: sqlite.Text(current.Cron), Timezone: sqlite.Text(current.Timezone), TimeoutSeconds: sqlite.Integer(current.TimeoutSeconds), Coalesce: sqlite.Integer(dbtypes.Boolean(current.Coalesce)), Enabled: sqlite.Integer(dbtypes.Boolean(current.Enabled)), UpdatedAt: sqlite.Number(nowSeconds()), ID: sqlite.Integer(input.TaskID)}); err != nil {
			return err
		}
		task, err = getTask(ctx, connection, input.TaskID)
		if err != nil {
			return err
		}
		if task != nil {
			return auditTarget(ctx, connection, audit, input.TaskID)
		}
		return nil
	})
	return task, err
}

// Delete preserves run history and audits only an actual deletion after live administrator validation.
func (repository *Repository) Delete(ctx context.Context, id int64, actor *identity.ID, audit *auditdomain.AuditEvent) (bool, error) {
	var didDelete bool
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		result, err := queries.DeleteTask(ctx, connection, schedulerdb.DeleteTaskParams{ID: sqlite.Integer(id)})
		if err != nil {
			return err
		}
		count, err := result.RowsAffected()
		if err != nil {
			return err
		}
		didDelete = count > 0
		if didDelete {
			return auditTarget(ctx, connection, audit, id)
		}
		return nil
	})
	return didDelete, err
}

// RecordTaskRun retains the legacy task-only terminal status operation.
func (repository *Repository) RecordTaskRun(ctx context.Context, id int64, status domain.State, ranAt float64) (bool, error) {
	if !status.IsTerminal() {
		return false, &domain.ValidationError{Message: "Scheduled task result must be terminal"}
	}
	var didUpdate bool
	err := repository.auth.WithConnection(ctx, func(connection *sql.Conn) error {
		var err error
		didUpdate, err = execute(queries.RecordTaskRun(ctx, connection, schedulerdb.RecordTaskRunParams{LastRunAt: new(sqlite.Number(ranAt)), LastStatus: sqlite.Text(status), UpdatedAt: sqlite.Number(nowSeconds()), ID: sqlite.Integer(id)}))
		return err
	})
	return didUpdate, err
}

func execute(result sql.Result, err error) (bool, error) {
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	return count > 0, err
}
