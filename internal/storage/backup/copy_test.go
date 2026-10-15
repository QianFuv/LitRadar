package backup

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	native "github.com/mattn/go-sqlite3"
)

// createCopyFixture builds a closed database large enough to require several native backup steps.
func createCopyFixture(testing testing.TB, megabytes int) (string, int) {
	testing.Helper()
	filename := filepath.Join(testing.TempDir(), "source.sqlite")
	database, err := storage.OpenPlain(filename)
	if err != nil {
		testing.Fatal(err)
	}
	defer database.Close()
	rows := megabytes * 16
	_, err = database.Exec(`CREATE TABLE payload(id INTEGER PRIMARY KEY, data BLOB NOT NULL);
WITH RECURSIVE entries(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM entries WHERE id<?)
INSERT INTO payload SELECT id,zeroblob(65536) FROM entries;`, rows)
	if err != nil {
		testing.Fatal(err)
	}
	return filename, rows
}

// TestBackupLockedSourceRetryAndCancellation exercises actual BUSY steps and destination cleanup.
func TestBackupLockedSourceRetryAndCancellation(t *testing.T) {
	for _, shouldCancel := range []bool{false, true} {
		t.Run(fmt.Sprintf("cancel=%t", shouldCancel), func(t *testing.T) {
			source, rows := createCopyFixture(t, 8)
			destination := filepath.Join(t.TempDir(), "snapshot.sqlite")
			writer, err := storage.OpenPlain(source)
			if err != nil {
				t.Fatal(err)
			}
			defer writer.Close()
			input, err := storage.OpenPlain(source)
			if err != nil {
				t.Fatal(err)
			}
			defer input.Close()
			output, err := storage.OpenPlain(destination)
			if err != nil {
				t.Fatal(err)
			}
			defer output.Close()
			ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
			defer cancel()
			sourceConnection, err := input.Conn(ctx)
			if err != nil {
				t.Fatal(err)
			}
			defer sourceConnection.Close()
			if _, err := sourceConnection.ExecContext(ctx, "PRAGMA busy_timeout=0"); err != nil {
				t.Fatal(err)
			}
			destinationConnection, err := output.Conn(ctx)
			if err != nil {
				t.Fatal(err)
			}
			defer destinationConnection.Close()
			err = sourceConnection.Raw(func(source any) error {
				return destinationConnection.Raw(func(destination any) error {
					backup, err := destination.(*native.SQLiteConn).Backup("main", source.(*native.SQLiteConn), "main")
					if err != nil {
						return err
					}
					if _, err := writer.Exec("BEGIN EXCLUSIVE"); err != nil {
						backup.Finish()
						return err
					}
					defer writer.Exec("ROLLBACK")
					if done, err := backup.Step(128); done || err != nil || backup.PageCount() != 0 {
						backup.Finish()
						t.Fatal("fixture did not produce a native BUSY step", done, err)
					}
					triggered := make(chan error, 1)
					timer := time.AfterFunc(60*time.Millisecond, func() {
						if shouldCancel {
							cancel()
							triggered <- nil
						} else {
							_, err := writer.Exec("ROLLBACK")
							triggered <- err
						}
					})
					failure := completeDatabaseBackup(ctx, backup)
					if timer.Stop() {
						t.Fatal("backup returned before the source was released or context canceled", failure)
					}
					if err := <-triggered; err != nil {
						return err
					}
					if shouldCancel && !errors.Is(failure, context.Canceled) || !shouldCancel && failure != nil {
						t.Fatal("backup lost retry/cancellation behavior", failure)
					}
					_, err = destination.(*native.SQLiteConn).Exec("CREATE TABLE released(value INTEGER)", nil)
					return err
				})
			})
			if err != nil {
				t.Fatal("backup did not finish or release its destination transaction", err)
			}
			destinationConnection.Close()
			output.Close()
			if !shouldCancel {
				assertCopiedPayload(t, destination, rows)
			}
		})
	}
}

// assertCopiedPayload verifies complete content, SQLite integrity, and the standalone output mode.
func assertCopiedPayload(testing testing.TB, filename string, expectedRows int) {
	testing.Helper()
	database, err := storage.OpenPlain(filename)
	if err != nil {
		testing.Fatal(err)
	}
	defer database.Close()
	var count, bytes int
	if err := database.QueryRow("SELECT count(*),sum(length(data)) FROM payload").Scan(&count, &bytes); err != nil || count != expectedRows || bytes != expectedRows*65536 {
		testing.Fatalf("incomplete payload: rows=%d bytes=%d error=%v", count, bytes, err)
	}
	for statement, expected := range map[string]string{"PRAGMA quick_check": "ok", "PRAGMA journal_mode": "delete"} {
		var result string
		if err := database.QueryRow(statement).Scan(&result); err != nil || result != expected {
			testing.Fatal(statement, result, err)
		}
	}
}

// BenchmarkCopyDatabase measures complete native copying while validating and removing outputs untimed.
func BenchmarkCopyDatabase(b *testing.B) {
	for _, megabytes := range []int{8, 32} {
		b.Run(fmt.Sprintf("%dMiB", megabytes), func(b *testing.B) {
			source, rows := createCopyFixture(b, megabytes)
			destination := filepath.Join(b.TempDir(), "snapshot.sqlite")
			b.SetBytes(int64(megabytes) << 20)
			b.ReportAllocs()
			for b.Loop() {
				if err := copyDatabase(context.Background(), source, destination); err != nil {
					b.Fatal(err)
				}
				b.StopTimer()
				assertCopiedPayload(b, destination, rows)
				if err := os.Remove(destination); err != nil {
					b.Fatal(err)
				}
				b.StartTimer()
			}
		})
	}
}
