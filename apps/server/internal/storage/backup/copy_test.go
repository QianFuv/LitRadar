package backup

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
	native "modernc.org/sqlite"
	sqlite3 "modernc.org/sqlite/lib"
)

// createCopyFixture builds a closed database large enough to require several native backup steps.
func createCopyFixture(testing testing.TB, megabytes int) (string, int) {
	testing.Helper()
	filename := filepath.Join(testing.TempDir(), "source.sqlite")
	database, err := storage.OpenPlain(filename)
	if err != nil {
		testing.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			testing.Errorf("database.Close: %v", err)
		}
	}()
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
			defer func() {
				if err := writer.Close(); err != nil {
					t.Errorf("writer.Close: %v", err)
				}
			}()
			input, err := storage.OpenPlain(source)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := input.Close(); err != nil {
					t.Errorf("input.Close: %v", err)
				}
			}()
			output, err := storage.OpenPlain(destination)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := output.Close(); err != nil {
					t.Errorf("output.Close: %v", err)
				}
			}()
			ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
			defer cancel()
			sourceConnection, err := input.Conn(ctx)
			if err != nil {
				t.Fatal(err)
			}
			defer func() {
				if err := sourceConnection.Close(); err != nil {
					t.Errorf("sourceConnection.Close: %v", err)
				}
			}()
			if _, err := sourceConnection.ExecContext(ctx, "PRAGMA busy_timeout=0"); err != nil {
				t.Fatal(err)
			}
			destinationURI, err := backupDestinationURI(destination)
			if err != nil {
				t.Fatal(err)
			}
			err = sourceConnection.Raw(func(source any) error {
				backup, err := source.(interface {
					NewBackup(string) (*native.Backup, error)
				}).NewBackup(destinationURI)
				if err != nil {
					return err
				}
				if _, err := writer.Exec("BEGIN EXCLUSIVE"); err != nil {
					if err := backup.Finish(); err != nil {
						t.Errorf("backup.Finish: %v", err)
					}
					return err
				}
				hasTransaction := true
				defer func() {
					if !hasTransaction {
						return
					}
					if _, err := writer.Exec("ROLLBACK"); err != nil {
						t.Errorf("writer.Exec: %v", err)
					}
				}()
				var busy *native.Error
				if more, err := backup.Step(128); more || !errors.As(err, &busy) || busy.Code()&255 != sqlite3.SQLITE_BUSY || backup.PageCount() != 0 {
					if err := backup.Finish(); err != nil {
						t.Errorf("backup.Finish: %v", err)
					}
					t.Fatal("fixture did not produce a native BUSY step", more, err)
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
				hasTransaction = shouldCancel
				if shouldCancel && !errors.Is(failure, context.Canceled) || !shouldCancel && failure != nil {
					t.Fatal("backup lost retry/cancellation behavior", failure)
				}
				_, err = output.Exec("CREATE TABLE released(value INTEGER)")
				return err
			})
			if err != nil {
				t.Fatal("backup did not finish or release its destination transaction", err)
			}
			if err := output.Close(); err != nil {
				t.Errorf("output.Close: %v", err)
			}
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
	defer func() {
		if err := database.Close(); err != nil {
			testing.Errorf("database.Close: %v", err)
		}
	}()
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
