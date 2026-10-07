package maintenance

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// writeMaintenanceFixture creates an isolated inventory entry.
func writeMaintenanceFixture(t *testing.T, directory, name string, content []byte) {
	t.Helper()
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(directory, name), content, 0600); err != nil {
		t.Fatal(err)
	}
}

// TestInspectDirectoryRetainsPartialInventoryBeforeSidecarChecks proves complete entry admission wins.
func TestInspectDirectoryRetainsPartialInventoryBeforeSidecarChecks(t *testing.T) {
	configuration := config.FromProjectRoot(t.TempDir())
	writeMaintenanceFixture(t, configuration.IndexDir, "a.sqlite", []byte("abc"))
	writeMaintenanceFixture(t, configuration.IndexDir, "a.sqlite-wal", []byte("nonempty"))
	writeMaintenanceFixture(t, configuration.IndexDir, "z.txt", []byte("unexpected"))
	snapshot, err := inspectDirectory(configuration)
	assertCode(t, err, "invalid_layout")
	if !strings.Contains(err.Error(), "unexpected file") {
		t.Fatal(err)
	}
	if len(snapshot.databases) != 1 || snapshot.bytes != 3 || snapshot.databases[0].name != "a.sqlite" {
		t.Fatalf("%+v", snapshot)
	}
	if err := os.Remove(filepath.Join(configuration.IndexDir, "z.txt")); err != nil {
		t.Fatal(err)
	}
	_, err = inspectDirectory(configuration)
	if err == nil || !strings.Contains(err.Error(), "non-empty SQLite wal") {
		t.Fatal(err)
	}
}

// twoDatabaseFixture supplies ordered, byte-identical supported source databases.
func twoDatabaseFixture(t *testing.T) config.Config {
	t.Helper()
	configuration := fixture(t, 9)
	writeMaintenanceFixture(t, configuration.IndexDir, "second.sqlite", readSource(t, configuration))
	return configuration
}

// removeSecondCandidate injects validation failure after the first candidate can pass.
func removeSecondCandidate(paths RecoveryPaths) hooks {
	return hooks{beforeValidation: func() error { return os.Remove(filepath.Join(paths.Staging, "second.sqlite")) }}
}

// TestMarkedFailureRetainsCompletedReports checks the private result boundary.
func TestMarkedFailureRetainsCompletedReports(t *testing.T) {
	configuration := twoDatabaseFixture(t)
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	source, err := inspectDirectory(configuration)
	if err != nil {
		t.Fatal(err)
	}
	for index := range source.databases {
		source.databases[index].version = 9
	}
	report, err := runMarked(context.Background(), configuration, paths, source, 1000, 123, removeSecondCandidate(paths))
	if err == nil {
		t.Fatal("missing second candidate accepted")
	}
	if len(report.Databases) != 1 || report.Databases[0].Database != "content.sqlite" || report.Outcome != "" {
		t.Fatalf("%+v", report)
	}
}

// TestPublicFailureDiscardsCompletedReports checks the public zero-report and cleanup boundary.
func TestPublicFailureDiscardsCompletedReports(t *testing.T) {
	configuration := twoDatabaseFixture(t)
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	report, err := optimize(context.Background(), Options{configuration, true}, 1000, removeSecondCandidate(paths))
	assertCode(t, err, "operation_failed")
	if report.Outcome != "" || report.Databases == nil || len(report.Databases) != 0 || report.SourceBytes != 0 {
		t.Fatalf("%+v", report)
	}
	requireMaintenancePresence(t, paths.Marker, true)
	requireMaintenancePresence(t, paths.Staging, false)
	requireMaintenancePresence(t, paths.Rollback, false)
}

// requireMaintenancePresence checks exact recovery artifact retention.
func requireMaintenancePresence(t *testing.T, filename string, expected bool) {
	t.Helper()
	present, err := exists(filename)
	if err != nil {
		t.Fatal(err)
	}
	if present != expected {
		t.Fatalf("%s present=%t, want %t", filename, present, expected)
	}
}

// createLateMaintenanceLease creates an active control lease without touching source databases.
func createLateMaintenanceLease(configuration config.Config) error {
	if err := os.MkdirAll(configuration.IndexControlDir, 0700); err != nil {
		return err
	}
	database, err := storage.Open(filepath.Join(configuration.IndexControlDir, "control.sqlite"), false, 1)
	if err != nil {
		return err
	}
	defer database.Close()
	_, err = database.Exec("CREATE TABLE index_batch_lease(lease_key INTEGER,expires_at INTEGER); INSERT INTO index_batch_lease VALUES(1,1001)")
	return err
}

// TestLateLeaseBlocksFirstRename checks the last activity gate after candidate validation.
func TestLateLeaseBlocksFirstRename(t *testing.T) {
	configuration := fixture(t, 9)
	original := readSource(t, configuration)
	hook := hooks{beforeValidation: func() error { return createLateMaintenanceLease(configuration) }}
	_, err := optimize(context.Background(), Options{configuration, true}, 1000, hook)
	assertCode(t, err, "operation_failed")
	if !strings.Contains(err.Error(), "active_lease") {
		t.Fatal(err)
	}
	if !bytes.Equal(original, readSource(t, configuration)) {
		t.Fatal("source changed before the first rename")
	}
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	requireMaintenancePresence(t, paths.Marker, true)
	requireMaintenancePresence(t, paths.Rollback, false)
}

// TestLateInventoryChangeBlocksFirstRename checks inventory comparison before the final activity gate.
func TestLateInventoryChangeBlocksFirstRename(t *testing.T) {
	configuration := fixture(t, 9)
	original := readSource(t, configuration)
	hook := hooks{beforeValidation: func() error { return os.WriteFile(filepath.Join(configuration.IndexDir, "new.sqlite"), nil, 0600) }}
	_, err := optimize(context.Background(), Options{configuration, true}, 1000, hook)
	assertCode(t, err, "operation_failed")
	if !strings.Contains(err.Error(), "source_changed") {
		t.Fatal(err)
	}
	if !bytes.Equal(original, readSource(t, configuration)) {
		t.Fatal("source changed before the first rename")
	}
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	requireMaintenancePresence(t, paths.Rollback, false)
}

// TestDirectRollbackClassificationControlsStagingCleanup preserves direct assertion semantics.
func TestDirectRollbackClassificationControlsStagingCleanup(t *testing.T) {
	for _, isWrapped := range []bool{false, true} {
		t.Run(fmt.Sprint(isWrapped), func(t *testing.T) { assertRollbackClassificationCleanup(t, isWrapped) })
	}
}

// assertRollbackClassificationCleanup distinguishes direct failures from wrapped failures.
func assertRollbackClassificationCleanup(t *testing.T, isWrapped bool) {
	t.Helper()
	configuration := fixture(t, 9)
	failure := error(Failure{Code: "rollback_failed", Message: "injected rollback classification"})
	if isWrapped {
		failure = fmt.Errorf("wrapped: %w", failure)
	}
	hook := hooks{beforeCopy: func(string) error { return failure }}
	report, err := optimize(context.Background(), Options{configuration, true}, 1000, hook)
	if err != failure {
		t.Fatalf("error identity changed: %v", err)
	}
	if report.Databases == nil || len(report.Databases) != 0 {
		t.Fatal(report)
	}
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	requireMaintenancePresence(t, paths.Marker, true)
	requireMaintenancePresence(t, paths.Staging, !isWrapped)
}

// TestFailedRenameCompensationRetainsOriginal checks pre-switch rollback hook failure.
func TestFailedRenameCompensationRetainsOriginal(t *testing.T) {
	configuration := fixture(t, 9)
	original := readSource(t, configuration)
	hook := hooks{afterRename: func() error { return errors.New("rename hook") }, beforeRollback: func() error { return errors.New("rollback hook") }}
	_, err := optimize(context.Background(), Options{configuration, true}, 1000, hook)
	assertCode(t, err, "rollback_failed")
	paths, err := Paths(configuration)
	if err != nil {
		t.Fatal(err)
	}
	retained, err := os.ReadFile(filepath.Join(paths.Rollback, "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(original, retained) {
		t.Fatal("retained original changed")
	}
	requireMaintenancePresence(t, configuration.IndexDir, false)
	requireMaintenancePresence(t, paths.Staging, true)
	requireMaintenancePresence(t, paths.Marker, true)
}

// TestKnownCleanupSkipsMissingPathsBeforeAdmission preserves the exact deletion boundary.
func TestKnownCleanupSkipsMissingPathsBeforeAdmission(t *testing.T) {
	root := t.TempDir()
	wrong := filepath.Join(root, "wrong")
	if err := removeKnown(wrong, root, stagingName); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(wrong, 0700); err != nil {
		t.Fatal(err)
	}
	assertCode(t, removeKnown(wrong, root, stagingName), "invalid_layout")
	requireMaintenancePresence(t, wrong, true)
	target := filepath.Join(root, "data", stagingName)
	writeMaintenanceFixture(t, filepath.Dir(target), stagingName, nil)
	assertCode(t, removeKnown(target, root, stagingName), "invalid_layout")
	if err := os.Remove(target); err != nil {
		t.Fatal(err)
	}
	if err := os.Mkdir(target, 0700); err != nil {
		t.Fatal(err)
	}
	if err := removeKnown(target, root, stagingName); err != nil {
		t.Fatal(err)
	}
	requireMaintenancePresence(t, target, false)
}
