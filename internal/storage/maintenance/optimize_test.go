package maintenance

import (
	"bytes"
	"context"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func fixture(t *testing.T, version int) config.Config {
	t.Helper()
	configuration := config.FromProjectRoot(t.TempDir())
	if err := os.MkdirAll(configuration.IndexDir, 0700); err != nil {
		t.Fatal(err)
	}
	source := filepath.Join("..", "..", "..", "tests", "migration", "storage", "fixtures", "content-v"+strconv.Itoa(version)+".sqlite.fixture")
	raw, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(configuration.IndexDir, "content.sqlite"), raw, 0600); err != nil {
		t.Fatal(err)
	}
	return configuration
}
func assertCode(t *testing.T, err error, code string) {
	t.Helper()
	var failure Failure
	if !errors.As(err, &failure) || failure.Code != code {
		t.Fatalf("want %s got %v", code, err)
	}
}
func readSource(t *testing.T, configuration config.Config) []byte {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join(configuration.IndexDir, "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	return raw
}

func TestOptimizeActualHistoricalSourcesIntoValidatedV9(t *testing.T) {
	for _, version := range []int{6, 7, 8, 9} {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			configuration := fixture(t, version)
			report, err := optimize(context.Background(), Options{configuration, true}, 1000, hooks{})
			if err != nil {
				t.Fatal(err)
			}
			if report.Outcome != "optimized" || report.DatabaseCount != 1 || report.Databases[0].SourceSchemaVersion != int64(version) || report.Databases[0].After.HasContentShadow || report.Databases[0].RowCounts["articles"] == 0 || len(report.Databases[0].RowCounts) != 9 {
				t.Fatalf("%+v", report)
			}
			if report.TemporaryBytesRequired != 2*report.SourceBytes+64*1024*1024 {
				t.Fatal(report)
			}
			if err := CheckInterrupted(configuration); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestConfirmationAndEmptyNoopPrecedeActivityChecks(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	if _, err := Optimize(context.Background(), Options{configuration, false}); err == nil {
		t.Fatal("unconfirmed allowed")
	} else {
		assertCode(t, err, "confirmation_required")
	}
	if err := os.MkdirAll(filepath.Dir(configuration.AuthDbPath), 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(configuration.AuthDbPath, []byte("not SQLite"), 0600); err != nil {
		t.Fatal(err)
	}
	report, err := Optimize(context.Background(), Options{configuration, true})
	if err != nil || report.Outcome != "noop" {
		t.Fatalf("%+v %v", report, err)
	}
}

func TestRecoveryArtifactsAndUnsafeLayoutsBlockBeforeCopy(t *testing.T) {
	for _, name := range []string{markerName, stagingName, rollbackName} {
		t.Run(name, func(t *testing.T) {
			configuration := fixture(t, 9)
			if err := os.WriteFile(filepath.Join(configuration.ProjectRoot, "data", name), []byte("malformed"), 0600); err != nil {
				t.Fatal(err)
			}
			_, err := Optimize(context.Background(), Options{configuration, true})
			assertCode(t, err, "interrupted_state")
		})
	}
	for _, name := range []string{"unexpected.txt", "orphan.sqlite-wal", "content.sqlite-wal", "content.sqlite-journal"} {
		t.Run(name, func(t *testing.T) {
			configuration := fixture(t, 9)
			if err := os.WriteFile(filepath.Join(configuration.IndexDir, name), []byte("nonempty"), 0600); err != nil {
				t.Fatal(err)
			}
			_, err := Optimize(context.Background(), Options{configuration, true})
			assertCode(t, err, "invalid_layout")
			if err := CheckInterrupted(configuration); err != nil {
				t.Fatal("invalid input acquired marker", err)
			}
		})
	}
}

func TestUnsupportedHistoricalVersionsDoNotMigrateSource(t *testing.T) {
	for _, version := range []int{4, 5} {
		t.Run(strconv.Itoa(version), func(t *testing.T) {
			configuration := fixture(t, version)
			before := readSource(t, configuration)
			_, err := Optimize(context.Background(), Options{configuration, true})
			assertCode(t, err, "unsupported_schema")
			if !bytes.Equal(before, readSource(t, configuration)) {
				t.Fatal("optimizer migrated source")
			}
			if err := CheckInterrupted(configuration); err != nil {
				t.Fatal(err)
			}
		})
	}
}

func TestCurrentAndProviderLeasesRejectBeforeMarker(t *testing.T) {
	for _, statement := range []string{"CREATE TABLE index_batch_lease(lease_key INTEGER,expires_at INTEGER); INSERT INTO index_batch_lease VALUES(1,1001);", "CREATE TABLE provider_leases(expires_at INTEGER); INSERT INTO provider_leases VALUES(1001);"} {
		configuration := fixture(t, 9)
		if err := os.MkdirAll(configuration.IndexControlDir, 0700); err != nil {
			t.Fatal(err)
		}
		database, err := storage.OpenPlain(filepath.Join(configuration.IndexControlDir, "lease.sqlite"))
		if err != nil {
			t.Fatal(err)
		}
		_, err = database.Exec(statement)
		database.Close()
		if err != nil {
			t.Fatal(err)
		}
		_, err = optimize(context.Background(), Options{configuration, true}, 1000, hooks{})
		assertCode(t, err, "active_lease")
		if err := EnsureInactive(context.Background(), configuration, 1001); err != nil {
			t.Fatal("expiry equality must be inactive", err)
		}
		if err := CheckInterrupted(configuration); err != nil {
			t.Fatal(err)
		}
	}
}

func TestFailuresRetainMarkerAndRestoreOriginalDirectory(t *testing.T) {
	for _, phase := range []string{"copy", "validation", "rename", "switch"} {
		t.Run(phase, func(t *testing.T) {
			configuration := fixture(t, 9)
			before := readSource(t, configuration)
			failure := func() error { return validation("content.sqlite", "injected fault") }
			hook := hooks{}
			switch phase {
			case "copy":
				hook.beforeCopy = func(string) error { return failure() }
			case "validation":
				hook.beforeValidation = failure
			case "rename":
				hook.afterRename = failure
			case "switch":
				hook.afterSwitch = failure
			}
			_, err := optimize(context.Background(), Options{configuration, true}, 1000, hook)
			assertCode(t, err, "operation_failed")
			if !bytes.Equal(before, readSource(t, configuration)) {
				t.Fatal("original source was not restored")
			}
			assertCode(t, CheckInterrupted(configuration), "interrupted_state")
			paths, err := Paths(configuration)
			if err != nil {
				t.Fatal(err)
			}
			if present, _ := exists(paths.Rollback); present {
				t.Fatal("rollback directory left after successful compensation")
			}
		})
	}
}

func TestRollbackFailureRetainsOriginalForRecovery(t *testing.T) {
	configuration := fixture(t, 9)
	before := readSource(t, configuration)
	hook := hooks{afterSwitch: func() error { return validation("content.sqlite", "post switch") }, beforeRollback: func() error { return errors.New("injected rollback failure") }}
	_, err := optimize(context.Background(), Options{configuration, true}, 1000, hook)
	assertCode(t, err, "rollback_failed")
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	saved, err := os.ReadFile(filepath.Join(paths.Rollback, "content.sqlite"))
	if err != nil || !bytes.Equal(before, saved) {
		t.Fatal("recovery source lost", err)
	}
	assertCode(t, CheckInterrupted(configuration), "interrupted_state")
}
