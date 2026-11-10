// Package telemetry migrates the independent, disposable anonymous load database.
package telemetry

import (
	"context"
	"fmt"
	"os"
	"path/filepath"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// Migrate admits the schema version before enabling writable storage policy.
func Migrate(ctx context.Context, filename string) error {
	if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
		return err
	}
	database, err := sqlite.OpenMigration(filename)
	if err != nil {
		return err
	}
	defer database.Close()
	var version int
	if err := database.QueryRowContext(ctx, "PRAGMA user_version").Scan(&version); err != nil {
		return err
	}
	if version > 1 {
		return fmt.Errorf("unsupported telemetry database schema version %d", version)
	}
	if version == 1 {
		return nil
	}
	transaction, err := database.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer transaction.Rollback()
	if _, err := transaction.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS hourly_load(hour INTEGER NOT NULL, dimensions TEXT NOT NULL, aggregate TEXT NOT NULL, PRIMARY KEY(hour,dimensions)) WITHOUT ROWID; PRAGMA user_version=1;`); err != nil {
		return err
	}
	return transaction.Commit()
}
