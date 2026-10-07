package index

import (
	"bytes"
	"context"
	"database/sql"
	contentfixture "github.com/QianFuv/LitRadar/internal/testkit/content"

	"errors"
	"os"
	"path/filepath"

	"strconv"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

type historical struct {
	Version int
}

func fixtures(t *testing.T) []historical {
	t.Helper()
	return []historical{{Version: 4}, {Version: 5}, {Version: 6}, {Version: 7}, {Version: 8}, {Version: 9}}
}

func copyFixture(t *testing.T, fixture historical) (string, []byte) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "content.sqlite")
	if err := contentfixture.Create(filename, fixture.Version); err != nil {
		t.Fatal(err)
	}
	data, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	return filename, data
}

func inspectConnection(t *testing.T, filename string, read func(*sql.Conn)) {
	t.Helper()
	database, err := storage.Open(filename, true, 1)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	connection, err := database.Conn(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	read(connection)
}

func stringsFromQuery(t *testing.T, connection *sql.Conn, statement string, args ...any) []string {
	t.Helper()
	rows, err := connection.QueryContext(context.Background(), statement, args...)
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	result := []string{}
	for rows.Next() {
		var value string
		if err := rows.Scan(&value); err != nil {
			t.Fatal(err)
		}
		result = append(result, value)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	return result
}

// TestFreshEmptyAndLegacyDatabaseEntrances covers admitted empty and rejected legacy files.
func TestFreshEmptyAndLegacyDatabaseEntrances(t *testing.T) {
	ctx := context.Background()
	for _, isEmptyFile := range []bool{false, true} {
		assertFreshDatabaseEntrance(t, ctx, isEmptyFile)
	}
	for _, version := range []int{0, 1, 2, 3, 10} {
		assertRejectedDatabaseEntrance(t, ctx, version)
	}
}

// assertFreshDatabaseEntrance checks missing and zero-byte databases reach the current version.
func assertFreshDatabaseEntrance(t *testing.T, ctx context.Context, isEmptyFile bool) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "nested", "content.sqlite")
	if isEmptyFile {
		if err := os.MkdirAll(filepath.Dir(filename), 0700); err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filename, nil, 0600); err != nil {
			t.Fatal(err)
		}
	}
	if result, err := Migrate(ctx, filename); err != nil || result != (Summary{0, 9}) {
		t.Fatalf("%+v %v", result, err)
	}
}

// assertRejectedDatabaseEntrance checks rejection leaves the exact historical file unchanged.
func assertRejectedDatabaseEntrance(t *testing.T, ctx context.Context, version int) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "legacy.sqlite")
	database, err := storage.OpenMigration(filename)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := database.Exec("CREATE TABLE legacy(value TEXT); PRAGMA user_version=" + strconv.Itoa(version)); err != nil {
		t.Fatal(err)
	}
	database.Close()
	before, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	_, failure := Migrate(ctx, filename)
	assertRejectedEntranceClassification(t, version, failure)
	after, err := os.ReadFile(filename)
	if err != nil || !bytes.Equal(before, after) {
		t.Fatal("rejected file changed")
	}
}

// assertRejectedEntranceClassification distinguishes unsupported future and rebuild entrances.
func assertRejectedEntranceClassification(t *testing.T, version int, failure error) {
	t.Helper()
	if failure == nil {
		t.Fatal("unsupported database accepted")
	}
	var future UnsupportedVersion
	var rebuild RebuildRequired
	if version == 10 && !errors.As(failure, &future) || version < 10 && !errors.As(failure, &rebuild) {
		t.Fatalf("wrong classification: %v", failure)
	}
}
