// Package cfp persists original CFP snapshots and generation-fenced refresh publication.
package cfp

import (
	"context"
	"database/sql"
	"errors"
	"sync"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	platform "github.com/QianFuv/LitRadar/internal/platform/sqlite"
)

var ErrStaleRefresh = errors.New("CFP refresh was superseded or expired")

type InvalidError struct{ Message string }

func (failure *InvalidError) Error() string { return failure.Message }
func invalid(message string) error          { return &InvalidError{Message: message} }

// Repository never creates parent directories or migrates the business database implicitly.
type Repository struct {
	database           *sql.DB
	snapshotMutex      sync.Mutex
	snapshotConnection *sql.Conn
	snapshotVersion    int64
	snapshot           []JournalSnapshot
}

func Open(path string) (*Repository, error) {
	database, err := platform.Open(platform.Config{Filename: path, Mode: "rwc", MaxConnections: 8})
	if err != nil {
		return nil, err
	}
	return &Repository{database: database}, nil
}

// Close releases the retained read connection before closing its owning pool.
func (repository *Repository) Close() error {
	repository.snapshotMutex.Lock()
	defer repository.snapshotMutex.Unlock()
	repository.clearSnapshot()
	return repository.database.Close()
}

// clearSnapshot discards connection-local version identity whenever its connection is released.
func (repository *Repository) clearSnapshot() {
	if repository.snapshotConnection != nil {
		repository.snapshotConnection.Close()
	}
	repository.snapshotConnection = nil
	repository.snapshot = nil
}

func (repository *Repository) transaction(ctx context.Context, isImmediate bool, operation func(*sql.Conn) error) (resultError error) {
	defer func() { resultError = databaseError(resultError) }()
	connection, err := repository.database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	return connectionTransaction(ctx, connection, isImmediate, operation)
}

// connectionTransaction keeps all statements, including cache validation, in one SQLite snapshot.
func connectionTransaction(ctx context.Context, connection *sql.Conn, isImmediate bool, operation func(*sql.Conn) error) error {
	begin := "BEGIN"
	if isImmediate {
		begin = "BEGIN IMMEDIATE"
	}
	if _, err := connection.ExecContext(ctx, begin); err != nil {
		return err
	}
	defer connection.ExecContext(context.Background(), "ROLLBACK")
	if err := operation(connection); err != nil {
		return err
	}
	_, err := connection.ExecContext(ctx, "COMMIT")
	return err
}

type ImportResult struct {
	DidImport   bool   `json:"didImport"`
	Journals    uint64 `json:"journals"`
	Notices     uint64 `json:"notices"`
	ContentHash string `json:"contentHash"`
}
type SourceStatus struct {
	SourceKey      string  `json:"sourceKey"`
	Status         string  `json:"status"`
	LastAttempt    *int64  `json:"lastAttempt"`
	LastSuccess    *int64  `json:"lastSuccess"`
	LastError      *string `json:"lastError"`
	Revision       int64   `json:"revision"`
	LeaseExpiresAt *int64  `json:"leaseExpiresAt"`
}
type JournalSnapshot struct {
	JournalKey      string
	CatalogIds      []string
	JournalTitle    string
	CheckedOn       string
	SourceUrl       *string
	SourceStatement *string
	Notices         []domain.Notice
	Sources         []SourceStatus
}
type RefreshLease struct {
	SourceKey  string
	Generation int64
	ExpiresAt  int64
}
type Publication struct {
	Capture       string
	CaptureFormat string
	SourceUrl     string
	ConfigVersion uint32
	Sources       []domain.Source
	EmptyJournals []domain.EmptyJournal
}
