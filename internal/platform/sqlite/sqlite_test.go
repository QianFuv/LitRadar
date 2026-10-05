package sqlite

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"

	sqlite3 "github.com/mattn/go-sqlite3"
)

func TestExistingOnlyModesNeverCreateMissingFile(t *testing.T) {
	for _, mode := range []string{"rw", "ro"} {
		t.Run(mode, func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "missing.sqlite")
			database, err := Open(Config{Filename: filename, Mode: mode, MaxConnections: 1})
			if err != nil {
				t.Fatal(err)
			}
			defer database.Close()
			if err := database.Ping(); err == nil {
				t.Fatal("missing existing-only database opened")
			}
			if _, err := os.Stat(filename); !os.IsNotExist(err) {
				t.Fatalf("existing-only mode created file: %v", err)
			}
		})
	}
}

func TestConcurrentPhysicalInitialization(t *testing.T) {
	database, err := Open(Config{Filename: filepath.Join(t.TempDir(), "simultaneous.sqlite"), Mode: "rwc", SimpleLibrary: trustedSimple(t), MaxConnections: 2})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	start := make(chan struct{})
	results := make(chan *sql.Conn, 2)
	errors := make(chan error, 2)
	var workers sync.WaitGroup
	for range 2 {
		workers.Go(func() {
			<-start
			connection, err := database.Conn(context.Background())
			results <- connection
			errors <- err
		})
	}
	close(start)
	workers.Wait()
	connections := []*sql.Conn{}
	for range 2 {
		connection := <-results
		if connection != nil {
			connections = append(connections, connection)
			defer connection.Close()
		}
	}
	for range 2 {
		if err := <-errors; err != nil {
			t.Fatal(err)
		}
	}
	for _, connection := range connections {
		var timeout, foreignKeys, synchronous int
		var journal, query string
		if err := connection.QueryRowContext(context.Background(), "PRAGMA busy_timeout").Scan(&timeout); err != nil {
			t.Fatal(err)
		}
		if err := connection.QueryRowContext(context.Background(), "PRAGMA foreign_keys").Scan(&foreignKeys); err != nil {
			t.Fatal(err)
		}
		if err := connection.QueryRowContext(context.Background(), "PRAGMA synchronous").Scan(&synchronous); err != nil {
			t.Fatal(err)
		}
		if err := connection.QueryRowContext(context.Background(), "PRAGMA journal_mode").Scan(&journal); err != nil {
			t.Fatal(err)
		}
		if err := connection.QueryRowContext(context.Background(), "SELECT simple_query('中文')").Scan(&query); err != nil {
			t.Fatal(err)
		}
		if timeout != 30000 || foreignKeys != 1 || synchronous != 1 || journal != "wal" || query == "" {
			t.Fatalf("connection policy %d %d %d %s %q", timeout, foreignKeys, synchronous, journal, query)
		}
	}
	if database.Stats().InUse != 2 {
		t.Fatal("experiment reused one physical connection")
	}
}

func trustedSimple(t *testing.T) string {
	t.Helper()
	name := "linux/libsimple.so"
	if runtime.GOOS == "windows" {
		name = "windows/simple.dll"
	}
	filename, err := filepath.Abs("../../../libs/simple/" + name)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filename); err != nil {
		t.Fatal(err)
	}
	return filename
}

func checkedConnection(t *testing.T, database *sql.DB) *sql.Conn {
	t.Helper()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { connection.Close() })
	return connection
}

func TestEveryPhysicalConnectionLoadsSimpleAndDisablesExtensionSql(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "中文 space # % ? &.sqlite")
	if runtime.GOOS == "windows" {
		filename = filepath.Join(root, "中文 space # % &.sqlite")
	}
	database, err := Open(Config{Filename: filename, Mode: "rwc", NoFollow: true, SimpleLibrary: trustedSimple(t), MaxConnections: 2})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	first := checkedConnection(t, database)
	second := checkedConnection(t, database)
	ctx := context.Background()
	if _, err := first.ExecContext(ctx, "CREATE VIRTUAL TABLE search USING fts5(text, tokenize='simple 0', content='', contentless_delete=1); INSERT INTO search(rowid,text) VALUES(1,'中文期刊 migration')"); err != nil {
		t.Fatal(err)
	}
	for _, connection := range []*sql.Conn{first, second} {
		var count, foreignKeys int
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM search WHERE search MATCH '中文'").Scan(&count); err != nil || count != 1 {
			t.Fatalf("Per-connection Simple failed: %d %v", count, err)
		}
		if err := connection.QueryRowContext(ctx, "PRAGMA foreign_keys").Scan(&foreignKeys); err != nil || foreignKeys != 1 {
			t.Fatalf("Foreign keys disabled: %d %v", foreignKeys, err)
		}
		if _, err := connection.ExecContext(ctx, "SELECT load_extension(?)", trustedSimple(t)); err == nil || !strings.Contains(err.Error(), "not authorized") {
			t.Fatalf("SQL extension loading must be disabled: %v", err)
		}
	}
	first.Close()
	second.Close()
	database.SetMaxIdleConns(0)
	third := checkedConnection(t, database)
	var count int
	if err := third.QueryRowContext(ctx, "SELECT count(*) FROM search WHERE search MATCH 'migration'").Scan(&count); err != nil || count != 1 {
		t.Fatalf("Recycled connection lost tokenizer: %d %v", count, err)
	}
	var version string
	if err := third.QueryRowContext(ctx, "SELECT sqlite_version()").Scan(&version); err != nil {
		t.Fatal(err)
	}
	t.Logf("native SQLite %s; %s/%s; extension %s", version, runtime.GOOS, runtime.GOARCH, trustedSimple(t))
	rows, err := third.QueryContext(ctx, "PRAGMA compile_options")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	for rows.Next() {
		var option string
		if err := rows.Scan(&option); err != nil {
			t.Fatal(err)
		}
		t.Log(option)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
}

func TestTransactionsAndBackupPreserveCommittedState(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	filename := filepath.Join(root, "auth.sqlite")
	database, err := Open(Config{Filename: filename, Mode: "rwc", NoFollow: true, MaxConnections: 2})
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec("CREATE TABLE scheduled_task_runs(id INTEGER PRIMARY KEY AUTOINCREMENT); INSERT INTO scheduled_task_runs VALUES(1000); DELETE FROM scheduled_task_runs"); err != nil {
		t.Fatal(err)
	}

	if _, err := database.Exec("CREATE TABLE atomic_test(id INTEGER PRIMARY KEY, value TEXT NOT NULL UNIQUE)"); err != nil {
		t.Fatal(err)
	}
	tx, err := database.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer tx.Rollback()
	if _, err := tx.Exec("INSERT INTO atomic_test VALUES(1,'first')"); err != nil {
		t.Fatal(err)
	}
	if _, err := tx.Exec("INSERT INTO atomic_test VALUES(2,'first')"); err == nil {
		t.Fatal("Expected uniqueness failure")
	}
	if err := tx.Rollback(); err != nil {
		t.Fatal(err)
	}
	var count int
	if err := database.QueryRow("SELECT count(*) FROM atomic_test").Scan(&count); err != nil || count != 0 {
		t.Fatalf("Atomic rollback failed: %d %v", count, err)
	}
	backup, err := Open(Config{Filename: filepath.Join(root, "backup.sqlite"), Mode: "rwc", NoFollow: true, MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer backup.Close()
	source := checkedConnection(t, database)
	destination := checkedConnection(t, backup)
	err = source.Raw(func(sourceDriver any) error {
		return destination.Raw(func(destinationDriver any) error {
			operation, err := destinationDriver.(*sqlite3.SQLiteConn).Backup("main", sourceDriver.(*sqlite3.SQLiteConn), "main")
			if err != nil {
				return err
			}
			isDone, err := operation.Step(-1)
			finishError := operation.Finish()
			if err != nil {
				return err
			}
			if !isDone {
				t.Error("Backup did not complete")
			}
			return finishError
		})
	})
	if err != nil {
		t.Fatal(err)
	}
	var sequence int64
	if err := destination.QueryRowContext(ctx, "SELECT seq FROM sqlite_sequence WHERE name='scheduled_task_runs'").Scan(&sequence); err != nil || sequence != 1000 {
		t.Fatalf("Backup sequence changed: %d %v", sequence, err)
	}
	if err := destination.QueryRowContext(ctx, "SELECT count(*) FROM atomic_test").Scan(&count); err != nil || count != 0 {
		t.Fatalf("Backup transaction contents changed: %d %v", count, err)
	}
}

func TestFileUriPreservesNamesAndReadModes(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "literal#% &中文.sqlite")
	encoded, err := FileUri(filename, "rwc")
	if err != nil || !strings.Contains(encoded, "%23%25") || strings.Contains(encoded, "#") {
		t.Fatalf("Unsafe URI %s %v", encoded, err)
	}
	database, err := Open(Config{Filename: filename, Mode: "rwc", NoFollow: true, MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("CREATE TABLE retained(value); INSERT INTO retained VALUES(42)"); err != nil {
		t.Fatal(err)
	}
	database.Close()
	readOnly, err := Open(Config{Filename: filename, Mode: "ro", NoFollow: true, MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer readOnly.Close()
	if _, err := readOnly.Exec("INSERT INTO retained VALUES(43)"); err == nil {
		t.Fatal("Read-only opened writable")
	}
	var count int
	if err := readOnly.QueryRow("SELECT count(*) FROM retained").Scan(&count); err != nil || count != 1 {
		t.Fatalf("Read-only changed target: %d %v", count, err)
	}
	entries, err := os.ReadDir(root)
	if err != nil {
		t.Fatal(err)
	}
	for _, entry := range entries {
		if !strings.HasPrefix(entry.Name(), filepath.Base(filename)) {
			t.Fatalf("Opened a different target: %s", entry.Name())
		}
	}
}
