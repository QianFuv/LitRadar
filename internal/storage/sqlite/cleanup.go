package sqlite

import (
	"context"
	"os"

	native "github.com/mattn/go-sqlite3"
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
func CleanupSidecars(ctx context.Context, filename string) (SidecarCleanup, error) {
	if !hasSidecars(filename) {
		return SidecarNotPresent, nil
	}
	database, err := open(filename, "rw", 1, &native.SQLiteDriver{})
	if err != nil {
		return "", err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return "", err
	}
	defer connection.Close()
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
