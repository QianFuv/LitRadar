package delivery

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"

	platform "github.com/QianFuv/LitRadar/apps/server/internal/platform/sqlite"
)

// Error exposes stable classifications without SQL or legacy payload contents.
type Error struct {
	Kind, Detail string
	cause        error
}

func (err *Error) Error() string {
	switch err.Kind {
	case "io":
		return "Delivery state filesystem access failed"
	case "json", "invalid_legacy":
		return "Legacy delivery state is invalid"
	case "input":
		return err.Detail
	case "stored":
		return "Stored delivery state is invalid"
	case "not_found":
		return "Delivery record not found"
	case "conflict":
		return "Delivery state changed concurrently"
	case "audit":
		return "Security audit persistence failed"
	case "legacy_conflict":
		return "Legacy delivery state changed after import"
	case "legacy_size":
		return "Legacy delivery state is too large"
	default:
		return "Delivery storage operation failed"
	}
}

// Unwrap preserves underlying error classification for bounded retry decisions.
func (err *Error) Unwrap() error { return err.cause }

// GoString excludes implementation details from diagnostic formatting.
func (err *Error) GoString() string { return err.Error() }

var ErrConflict = &Error{Kind: "conflict"}
var ErrNotFound = &Error{Kind: "not_found"}

func storageError(err error) error {
	if err == nil {
		return nil
	}
	if _, isClassified := err.(*Error); isClassified {
		return err
	}
	return &Error{Kind: "sqlite", cause: err}
}

func requireRecord[T any](record *T, err error) (*T, error) {
	if err != nil {
		return nil, err
	}
	if record == nil {
		return nil, ErrNotFound
	}
	return record, nil
}

func invalid(detail string) error { return &Error{Kind: "input", Detail: detail} }

// Repository opens existing migrated auth schemas without running migrations.
type Repository struct{ database *sql.DB }

// Open establishes ordinary SQLite connection policy and creates missing parent directories.
func Open(filename string) (*Repository, error) {
	if err := os.MkdirAll(filepath.Dir(filename), 0777); err != nil {
		return nil, &Error{Kind: "io", cause: err}
	}
	database, err := platform.Open(platform.Config{Filename: filename, Mode: "rwc", MaxConnections: 8})
	if err != nil {
		return nil, storageError(err)
	}
	return &Repository{database}, nil
}

// Close releases physical connections.
func (repository *Repository) Close() error { return storageError(repository.database.Close()) }

func (repository *Repository) immediate(ctx context.Context, mutate func(*sql.Conn) error) (resultError error) {
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return storageError(err)
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return storageError(err)
	}
	hasTransaction := true
	defer func() {
		if hasTransaction {
			if _, rollbackError := connection.ExecContext(context.Background(), "ROLLBACK"); rollbackError != nil {
				slog.Error("storage.rollback.failed", "error", rollbackError)
			}
		}
	}()
	if err := mutate(connection); err != nil {
		return storageError(err)
	}
	_, err = connection.ExecContext(ctx, "COMMIT")
	hasTransaction = err != nil
	return storageError(err)
}

func executeCas(ctx context.Context, connection *sql.Conn, query string, args ...any) error {
	return checkCAS(connection.ExecContext(ctx, query, args...))
}

func checkCAS(result sql.Result, err error) error {
	if err != nil {
		return storageError(err)
	}
	count, err := result.RowsAffected()
	if err != nil {
		return storageError(err)
	}
	if count != 1 {
		return ErrConflict
	}
	return nil
}
