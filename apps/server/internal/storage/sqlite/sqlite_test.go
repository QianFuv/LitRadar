package sqlite

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"
)

// TestProductionBuildProvidesFtsAndStorageMeasurement checks native capabilities and the plain storage role.
func TestProductionBuildProvidesFtsAndStorageMeasurement(t *testing.T) {
	database, err := OpenPlain(filepath.Join(t.TempDir(), "capabilities.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	}()
	assertStorageCapabilities(t, database)
	assertPlainConnectionRole(t, database)
}

// TestSidecarCleanupPreservesActiveWriterAndCleansIdleReader checks cleanup across explicit owner closes.
func TestSidecarCleanupPreservesActiveWriterAndCleansIdleReader(t *testing.T) {
	ctx := context.Background()
	filename := filepath.Join(t.TempDir(), "active.sqlite")
	if outcome, err := CleanupSidecars(ctx, filename); err != nil || outcome != SidecarNotPresent {
		t.Fatalf("%s %v", outcome, err)
	}
	database, err := Open(filename, false, 1)
	if err != nil {
		t.Fatal(err)
	}
	connection, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	assertActiveWriterSidecars(t, ctx, filename, connection)
	if err := connection.Close(); err != nil {
		t.Fatalf("connection.Close: %v", err)
	}
	if err := database.Close(); err != nil {
		t.Fatalf("database.Close: %v", err)
	}
	assertIdleReaderContents(t, ctx, filename)
	if outcome, err := CleanupSidecars(ctx, filename); err != nil || outcome != SidecarCleaned {
		t.Fatalf("idle reader: %s %v", outcome, err)
	}
	if hasSidecars(filename) {
		t.Fatal("idle sidecars remain")
	}
}

// assertStorageCapabilities retains build-option, FTS5 insertion and actual DBSTAT measurement assertions.
func assertStorageCapabilities(t *testing.T, database *sql.DB) {
	t.Helper()
	var enabled int
	if err := database.QueryRow("SELECT sqlite_compileoption_used('ENABLE_DBSTAT_VTAB')").Scan(&enabled); err != nil || enabled != 1 {
		t.Fatalf("SQLite driver lacks ENABLE_DBSTAT_VTAB: %d %v", enabled, err)
	}
	if _, err := database.Exec("CREATE VIRTUAL TABLE search USING fts5(text); INSERT INTO search VALUES('test')"); err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("CREATE VIRTUAL TABLE contentless USING fts5(text, content='', contentless_delete=1); CREATE VIRTUAL TABLE vocabulary USING fts5vocab(contentless, 'instance'); INSERT INTO contentless(rowid,text) VALUES(1,'driver capability')"); err != nil {
		t.Fatal(err)
	}
	var terms int
	if err := database.QueryRow("SELECT count(*) FROM vocabulary").Scan(&terms); err != nil || terms != 2 {
		t.Fatalf("fts5vocab unavailable: %d %v", terms, err)
	}
	if _, err := database.Exec("DELETE FROM contentless WHERE rowid=1"); err != nil {
		t.Fatalf("contentless_delete unavailable: %v", err)
	}
	var size int64
	if err := database.QueryRow("SELECT sum(pgsize) FROM dbstat").Scan(&size); err != nil || size == 0 {
		t.Fatalf("dbstat unavailable: %d %v", size, err)
	}
}

// assertPlainConnectionRole checks every original timeout, constraint, sync and journal expectation.
func assertPlainConnectionRole(t *testing.T, database *sql.DB) {
	t.Helper()
	var timeout, foreignKeys, synchronous int
	var mode string
	for pragma, destination := range map[string]*int{"busy_timeout": &timeout, "foreign_keys": &foreignKeys, "synchronous": &synchronous} {
		if err := database.QueryRow("PRAGMA " + pragma).Scan(destination); err != nil {
			t.Fatal(err)
		}
	}
	if err := database.QueryRow("PRAGMA journal_mode").Scan(&mode); err != nil {
		t.Fatal(err)
	}
	if timeout != 5000 || foreignKeys != 0 || synchronous != 2 || mode != "delete" {
		t.Fatalf("plain read role changed: timeout=%d FK=%d sync=%d mode=%s", timeout, foreignKeys, synchronous, mode)
	}
}

// assertActiveWriterSidecars keeps the writer alive through busy/presence checks and rollback.
func assertActiveWriterSidecars(t *testing.T, ctx context.Context, filename string, connection *sql.Conn) {
	t.Helper()
	if _, err := connection.ExecContext(ctx, "CREATE TABLE item(id INTEGER PRIMARY KEY); BEGIN IMMEDIATE; INSERT INTO item DEFAULT VALUES;"); err != nil {
		t.Fatal(err)
	}
	if outcome, err := CleanupSidecars(ctx, filename); err != nil || outcome != SidecarBusy {
		t.Fatalf("active writer: %s %v", outcome, err)
	}
	if !hasSidecars(filename) {
		t.Fatal("active sidecars deleted")
	}
	if _, err := connection.ExecContext(ctx, "ROLLBACK; INSERT INTO item DEFAULT VALUES"); err != nil {
		t.Fatal(err)
	}
}

// assertIdleReaderContents closes the readonly owner before the final cleanup checks.
func assertIdleReaderContents(t *testing.T, ctx context.Context, filename string) {
	t.Helper()
	reader, err := Open(filename, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	var count int
	if err := reader.QueryRowContext(ctx, "SELECT count(*) FROM item").Scan(&count); err != nil || count != 1 {
		t.Fatal(err)
	}
	if err := reader.Close(); err != nil {
		t.Fatalf("reader.Close: %v", err)
	}
}
