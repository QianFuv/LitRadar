package sqlite

import (
	"context"
	"database/sql"
	"errors"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"sync"
	"testing"

	sqlite3 "modernc.org/sqlite"
)

func TestExistingOnlyModesNeverCreateMissingFile(t *testing.T) {
	for _, mode := range []string{"rw", "ro"} {
		t.Run(mode, func(t *testing.T) {
			filename := filepath.Join(t.TempDir(), "missing.sqlite")
			database, err := Open(Config{Filename: filename, Mode: mode, MaxConnections: 1})
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := database.Close(); err != nil {
					t.Errorf("database.Close: %v", err)
				}
			}()
			if err := database.Ping(); err == nil {
				t.Fatal("missing existing-only database opened")
			}
			if _, err := os.Stat(filename); !os.IsNotExist(err) {
				t.Fatalf("existing-only mode created file: %v", err)
			}
		})
	}
}

// TestConcurrentPhysicalInitialization retains simultaneous physical opens and distinct live connection ownership.
func TestConcurrentPhysicalInitialization(t *testing.T) {
	database, err := Open(Config{Filename: filepath.Join(t.TempDir(), "simultaneous.sqlite"), Mode: "rwc", MaxConnections: 2})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	}()
	results, connectionErrors := startPhysicalConnections(database)
	connections := []*sql.Conn{}
	for range 2 {
		connection := <-results
		if connection != nil {
			connections = append(connections, connection)
			defer func() {
				if err := connection.Close(); err != nil && !errors.Is(err, sql.ErrConnDone) {
					t.Errorf("connection.Close: %v", err)
				}
			}()
		}
	}
	for range 2 {
		if err := <-connectionErrors; err != nil {
			t.Fatal(err)
		}
	}
	for _, connection := range connections {
		assertPhysicalConnectionPolicy(t, connection)
	}
	if database.Stats().InUse != 2 {
		t.Fatal("experiment reused one physical connection")
	}
}

func checkedConnection(t *testing.T, database *sql.DB) *sql.Conn {
	t.Helper()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := connection.Close(); err != nil && !errors.Is(err, sql.ErrConnDone) {
			t.Errorf("connection.Close: %v", err)
		}
	})
	return connection
}

// TestEveryPhysicalConnectionSearchesAndDisablesExtensionSql checks every open and recreation against built-in tokenizer policy.
func TestEveryPhysicalConnectionSearchesAndDisablesExtensionSql(t *testing.T) {
	root := t.TempDir()
	filename := filepath.Join(root, "中文 space # % ? &.sqlite")
	if runtime.GOOS == "windows" {
		filename = filepath.Join(root, "中文 space # % &.sqlite")
	}
	database, err := Open(Config{Filename: filename, Mode: "rwc", NoFollow: true, MaxConnections: 2})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	})
	first := checkedConnection(t, database)
	second := checkedConnection(t, database)
	ctx := context.Background()
	if _, err := first.ExecContext(ctx, "CREATE VIRTUAL TABLE search USING fts5(text, tokenize='ascii', content='', contentless_delete=1); INSERT INTO search(rowid,text) VALUES(1,'中 文 期 刊 migration')"); err != nil {
		t.Fatal(err)
	}
	for _, connection := range []*sql.Conn{first, second} {
		assertPhysicalTokenizer(t, ctx, connection)
	}
	if err := first.Close(); err != nil {
		t.Fatalf("first.Close: %v", err)
	}
	if err := second.Close(); err != nil {
		t.Fatalf("second.Close: %v", err)
	}
	database.SetMaxIdleConns(0)
	third := checkedConnection(t, database)
	var count int
	if err := third.QueryRowContext(ctx, "SELECT count(*) FROM search WHERE search MATCH 'migration'").Scan(&count); err != nil || count != 1 {
		t.Fatalf("Recycled connection lost tokenizer: %d %v", count, err)
	}
	logNativeCapabilities(t, ctx, third)
}

// TestTransactionsAndBackupPreserveCommittedState retains rollback, borrowed backup and destination-state ordering.
func TestTransactionsAndBackupPreserveCommittedState(t *testing.T) {
	ctx := context.Background()
	root := t.TempDir()
	filename := filepath.Join(root, "auth.sqlite")
	database, err := Open(Config{Filename: filename, Mode: "rwc", NoFollow: true, MaxConnections: 2})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	}()
	seedBackupSource(t, database)
	tx, err := database.BeginTx(ctx, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := tx.Rollback(); err != nil && !errors.Is(err, sql.ErrTxDone) {
			t.Errorf("tx.Rollback: %v", err)
		}
	}()
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
	defer func() {
		if err := backup.Close(); err != nil {
			t.Errorf("backup.Close: %v", err)
		}
	}()
	source := checkedConnection(t, database)
	destination := checkedConnection(t, backup)
	if err := destination.Close(); err != nil {
		t.Fatalf("destination.Close: %v", err)
	}
	err = backupPhysicalConnections(t, source, filepath.Join(root, "backup.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	destination = checkedConnection(t, backup)
	assertBackupState(t, ctx, destination)
}

// TestFileUriPreservesNamesAndReadModes checks escaped names and readonly policy on the same target.
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
	if err := database.Close(); err != nil {
		t.Fatalf("database.Close: %v", err)
	}
	readOnly, err := Open(Config{Filename: filename, Mode: "ro", NoFollow: true, MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := readOnly.Close(); err != nil {
			t.Errorf("readOnly.Close: %v", err)
		}
	}()
	assertReadOnlyTarget(t, root, filename, readOnly)
}

// startPhysicalConnections releases concurrent requests together and waits before result admission.
func startPhysicalConnections(database *sql.DB) (chan *sql.Conn, chan error) {
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
	return results, errors
}

// assertPhysicalConnectionPolicy checks every original physical initialization policy in order.
func assertPhysicalConnectionPolicy(t *testing.T, connection *sql.Conn) {
	t.Helper()
	var timeout, foreignKeys, synchronous int
	var journal string
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
	if timeout != 30000 || foreignKeys != 1 || synchronous != 1 || journal != "wal" {
		t.Fatalf("connection policy %d %d %d %s", timeout, foreignKeys, synchronous, journal)
	}
}

// assertPhysicalTokenizer checks search, constraints and disabled SQL extension loading on a live owner.
func assertPhysicalTokenizer(t *testing.T, ctx context.Context, connection *sql.Conn) {
	t.Helper()
	var count, foreignKeys int
	if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM search WHERE search MATCH '\"中 文\"'").Scan(&count); err != nil || count != 1 {
		t.Fatalf("Per-connection ASCII search failed: %d %v", count, err)
	}
	if err := connection.QueryRowContext(ctx, "PRAGMA foreign_keys").Scan(&foreignKeys); err != nil || foreignKeys != 1 {
		t.Fatalf("Foreign keys disabled: %d %v", foreignKeys, err)
	}
	if _, err := connection.ExecContext(ctx, "SELECT load_extension(?)", "untrusted-extension"); err == nil || !strings.Contains(err.Error(), "not authorized") {
		t.Fatalf("SQL extension loading must be disabled: %v", err)
	}
}

// logNativeCapabilities retains version identity and all compile-option row checks.
func logNativeCapabilities(t *testing.T, ctx context.Context, third *sql.Conn) {
	t.Helper()
	var version string
	if err := third.QueryRowContext(ctx, "SELECT sqlite_version()").Scan(&version); err != nil {
		t.Fatal(err)
	}
	t.Logf("native SQLite %s; %s/%s; built-in ASCII tokenizer", version, runtime.GOOS, runtime.GOARCH)
	rows, err := third.QueryContext(ctx, "PRAGMA compile_options")
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := rows.Close(); err != nil {
			t.Errorf("rows.Close: %v", err)
		}
	}()
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

// seedBackupSource retains sequence allocation and table creation before the atomic transaction.
func seedBackupSource(t *testing.T, database *sql.DB) {
	t.Helper()
	if _, err := database.Exec("CREATE TABLE scheduled_task_runs(id INTEGER PRIMARY KEY AUTOINCREMENT); INSERT INTO scheduled_task_runs VALUES(1000); DELETE FROM scheduled_task_runs"); err != nil {
		t.Fatal(err)
	}

	if _, err := database.Exec("CREATE TABLE atomic_test(id INTEGER PRIMARY KEY, value TEXT NOT NULL UNIQUE)"); err != nil {
		t.Fatal(err)
	}
}

// backupPhysicalConnections keeps the source borrowed while modernc owns the destination.
func backupPhysicalConnections(t *testing.T, source *sql.Conn, destination string) error {
	t.Helper()
	uri, err := FileUri(destination, "rwc")
	if err != nil {
		return err
	}
	return source.Raw(func(sourceDriver any) error {
		operation, err := sourceDriver.(interface {
			NewBackup(string) (*sqlite3.Backup, error)
		}).NewBackup(uri)
		if err != nil {
			return err
		}
		hasMore, err := operation.Step(-1)
		finishError := operation.Finish()
		if err != nil {
			return err
		}
		if hasMore {
			t.Error("Backup did not complete")
		}
		return finishError
	})
}

// assertBackupState checks sequence retention and exclusion of the rolled-back transaction.
func assertBackupState(t *testing.T, ctx context.Context, destination *sql.Conn) {
	t.Helper()
	var count int
	var sequence int64
	if err := destination.QueryRowContext(ctx, "SELECT seq FROM sqlite_sequence WHERE name='scheduled_task_runs'").Scan(&sequence); err != nil || sequence != 1000 {
		t.Fatalf("Backup sequence changed: %d %v", sequence, err)
	}
	if err := destination.QueryRowContext(ctx, "SELECT count(*) FROM atomic_test").Scan(&count); err != nil || count != 0 {
		t.Fatalf("Backup transaction contents changed: %d %v", count, err)
	}
}

// assertReadOnlyTarget checks write rejection, retained rows and every directory entry on the open reader.
func assertReadOnlyTarget(t *testing.T, root, filename string, readOnly *sql.DB) {
	t.Helper()
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
