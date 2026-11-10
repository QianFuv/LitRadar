package telemetry

import (
	"context"
	"path/filepath"
	"testing"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

// TestMigrateIsRepeatableAndRejectsFutureVersions protects independently versioned load data.
func TestMigrateIsRepeatableAndRejectsFutureVersions(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "telemetry.sqlite")
	for range 2 {
		if err := Migrate(context.Background(), filename); err != nil {
			t.Fatal(err)
		}
	}
	database, err := sqlite.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec("PRAGMA user_version=2"); err != nil {
		t.Fatal(err)
	}
	if err := Migrate(context.Background(), filename); err == nil {
		t.Fatal("future schema accepted")
	}
	var version int
	if err := database.QueryRow("PRAGMA user_version").Scan(&version); err != nil || version != 2 {
		t.Fatal(version, err)
	}
}
