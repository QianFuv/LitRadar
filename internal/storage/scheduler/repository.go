// Package scheduler persists task definitions and durable scheduled/manual claims.
package scheduler

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	auditdomain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/sqlite"
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
func authorize(ctx context.Context, connection *sql.Conn, actor *identity.Id) error {
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
	copy.TargetId = &id
	return auth.InsertAudit(ctx, connection, &copy)
}

type scanner interface{ Scan(...any) error }
type optionalNumber struct{ Value *float64 }

func (value *optionalNumber) Scan(raw any) error {
	if raw == nil {
		value.Value = nil
		return nil
	}
	var number sqlite.Number
	if err := number.Scan(raw); err != nil {
		return err
	}
	result := float64(number)
	value.Value = &result
	return nil
}

type unsignedInteger uint64

func (value *unsignedInteger) Scan(raw any) error {
	var integer sqlite.Integer
	if err := integer.Scan(raw); err != nil {
		return err
	}
	if integer < 0 {
		return errors.New("invalid SQLite unsigned integer column")
	}
	*value = unsignedInteger(integer)
	return nil
}

type jobColumn struct{ Value *domain.Job }

func (value *jobColumn) Scan(raw any) error {
	var text sqlite.OptionalText
	if err := text.Scan(raw); err != nil {
		return err
	}
	if text.Value == nil {
		value.Value = nil
		return nil
	}
	var job domain.Job
	if err := json.Unmarshal([]byte(*text.Value), &job); err != nil {
		return err
	}
	value.Value = &job
	return nil
}

const taskColumns = `id,name,job_spec,legacy_command,cron,timezone,timeout_seconds,coalesce,enabled,last_run_at,last_status,created_at,updated_at`

func scanTask(row scanner) (*domain.Task, error) {
	var id, coalesce, enabled sqlite.Integer
	var name, cron, timezone, status sqlite.Text
	var job jobColumn
	var legacy sqlite.OptionalText
	var timeout unsignedInteger
	var last optionalNumber
	var created, updated sqlite.Number
	if err := row.Scan(&id, &name, &job, &legacy, &cron, &timezone, &timeout, &coalesce, &enabled, &last, &status, &created, &updated); err != nil {
		return nil, err
	}
	return &domain.Task{Id: int64(id), Name: string(name), Job: job.Value, LegacyCommand: legacy.Value, Cron: string(cron), Timezone: string(timezone), TimeoutSeconds: uint64(timeout), Coalesce: coalesce != 0, Enabled: enabled != 0, LastRunAt: last.Value, LastStatus: domain.PersistedState(string(status)), CreatedAt: float64(created), UpdatedAt: float64(updated)}, nil
}

func getTask(ctx context.Context, connection *sql.Conn, id int64) (*domain.Task, error) {
	task, err := scanTask(connection.QueryRowContext(ctx, "SELECT "+taskColumns+" FROM scheduled_tasks WHERE id=?", id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return task, err
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
		rows, err := connection.QueryContext(ctx, "SELECT "+taskColumns+" FROM scheduled_tasks ORDER BY created_at DESC")
		if err != nil {
			return err
		}
		defer rows.Close()
		for rows.Next() {
			task, err := scanTask(rows)
			if err != nil {
				return err
			}
			tasks = append(tasks, *task)
		}
		return rows.Err()
	})
	return tasks, err
}

// Create validates typed arguments before atomically checking authority, inserting and auditing.
func (repository *Repository) Create(ctx context.Context, input domain.Create, actor *identity.Id, audit *auditdomain.AuditEvent) (*domain.Task, error) {
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
		result, err := connection.ExecContext(ctx, `INSERT INTO scheduled_tasks(name,job_spec,legacy_command,cron,timezone,timeout_seconds,coalesce,enabled,last_run_at,last_status,created_at,updated_at) VALUES(?,?,NULL,?,?,?,?,?,NULL,'idle',?,?)`, input.Name, string(encoded), input.Cron, input.Timezone, input.TimeoutSeconds, input.Coalesce, input.Enabled, now, now)
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
func (repository *Repository) Update(ctx context.Context, input domain.Update, actor *identity.Id, audit *auditdomain.AuditEvent) (*domain.Task, error) {
	var task *domain.Task
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		current, err := getTask(ctx, connection, input.TaskId)
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
		var encoded any
		if current.Job != nil {
			value, err := current.Job.MarshalJSON()
			if err != nil {
				return err
			}
			encoded = string(value)
		}
		if _, err := connection.ExecContext(ctx, `UPDATE scheduled_tasks SET name=?,job_spec=?,legacy_command=?,cron=?,timezone=?,timeout_seconds=?,coalesce=?,enabled=?,updated_at=? WHERE id=?`, current.Name, encoded, current.LegacyCommand, current.Cron, current.Timezone, current.TimeoutSeconds, current.Coalesce, current.Enabled, nowSeconds(), input.TaskId); err != nil {
			return err
		}
		task, err = getTask(ctx, connection, input.TaskId)
		if err != nil {
			return err
		}
		if task != nil {
			return auditTarget(ctx, connection, audit, input.TaskId)
		}
		return nil
	})
	return task, err
}

// Delete preserves run history and audits only an actual deletion after live administrator validation.
func (repository *Repository) Delete(ctx context.Context, id int64, actor *identity.Id, audit *auditdomain.AuditEvent) (bool, error) {
	var didDelete bool
	err := repository.auth.Immediate(ctx, false, func(connection *sql.Conn) error {
		if err := authorize(ctx, connection, actor); err != nil {
			return err
		}
		result, err := connection.ExecContext(ctx, "DELETE FROM scheduled_tasks WHERE id=?", id)
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
		didUpdate, err = execute(ctx, connection, `UPDATE scheduled_tasks SET last_run_at=?,last_status=?,updated_at=? WHERE id=?`, ranAt, status, nowSeconds(), id)
		return err
	})
	return didUpdate, err
}

func execute(ctx context.Context, connection *sql.Conn, query string, args ...any) (bool, error) {
	result, err := connection.ExecContext(ctx, query, args...)
	if err != nil {
		return false, err
	}
	count, err := result.RowsAffected()
	return count > 0, err
}
