package sqlite

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"os"

	native "modernc.org/sqlite"
)

// SidecarCleanup describes a non-blocking checkpoint; SQLite retains ownership of WAL and shared memory.
type SidecarCleanup string

const (
	SidecarNotPresent SidecarCleanup = "not_present"
	SidecarCleaned    SidecarCleanup = "cleaned"
	SidecarBusy       SidecarCleanup = "busy"
	SidecarRetained   SidecarCleanup = "retained"
)

func hasSidecars(filename string) bool {
	for _, suffix := range []string{"-wal", "-shm"} {
		if _, err := os.Stat(filename + suffix); err == nil {
			return true
		}
	}
	return false
}

// CleanupSidecars asks SQLite to remove idle sidecars on final close without directly deleting active state.
func CleanupSidecars(ctx context.Context, filename string) (resultValue1 SidecarCleanup, resultError error) {
	if !hasSidecars(filename) {
		return SidecarNotPresent, nil
	}
	database, err := open(filename, "rw", 1, &native.Driver{})
	if err != nil {
		return "", err
	}
	defer func() {
		if closeError := database.Close(); closeError != nil {
			resultError = errors.Join(resultError, fmt.Errorf("close database: %w", closeError))
		}
	}()
	connection, err := database.Conn(ctx)
	if err != nil {
		return "", err
	}
	defer func() {
		if closeError := connection.Close(); closeError != nil && !errors.Is(closeError, sql.ErrConnDone) {
			resultError = errors.Join(resultError, fmt.Errorf("close connection: %w", closeError))
		}
	}()
	if _, err := connection.ExecContext(ctx, "PRAGMA busy_timeout=0"); err != nil {
		return "", err
	}
	var busy, frames, checkpointed int64
	if err := connection.QueryRowContext(ctx, "PRAGMA wal_checkpoint(TRUNCATE)").Scan(&busy, &frames, &checkpointed); err != nil {
		return "", err
	}
	if err := connection.Close(); err != nil {
		return "", err
	}
	if err := database.Close(); err != nil {
		return "", err
	}
	if !hasSidecars(filename) {
		return SidecarCleaned, nil
	}
	if busy != 0 {
		return SidecarBusy, nil
	}
	return SidecarRetained, nil
}
