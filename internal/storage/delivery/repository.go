package delivery

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"

	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
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

func (repository *Repository) immediate(ctx context.Context, mutate func(*sql.Conn) error) error {
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return storageError(err)
	}
	defer connection.Close()
	if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
		return storageError(err)
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	if err := mutate(connection); err != nil {
		return storageError(err)
	}
	_, err = connection.ExecContext(ctx, "COMMIT")
	return storageError(err)
}

func executeCas(ctx context.Context, connection *sql.Conn, query string, args ...any) error {
	result, err := connection.ExecContext(ctx, query, args...)
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
