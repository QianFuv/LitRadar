package indexschema

import (
	"bytes"
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func TestFrozenContentSchemaValidatesWithoutChangingDatabase(t *testing.T) {
	fixture, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "data", "migration", "rust", "content.sqlite.fixture"))
	if err != nil {
		t.Fatal(err)
	}
	filename := filepath.Join(t.TempDir(), "content.sqlite")
	if err := os.WriteFile(filename, fixture, 0600); err != nil {
		t.Fatal(err)
	}
	database, err := storage.Open(filename, true, 2)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	ctx := context.Background()
	first, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer first.Close()
	second, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer second.Close()
	for _, connection := range []*sql.Conn{first, second} {
		if err := Validate(ctx, connection, Version); err != nil {
			t.Fatal(err)
		}
		if _, err := connection.ExecContext(ctx, "SELECT load_extension('untrusted')"); err == nil {
			t.Fatal("extension SQL remained enabled")
		}
	}
	current, err := os.ReadFile(filename)
	if err != nil || !bytes.Equal(fixture, current) {
		t.Fatal("read-only structure check changed canonical bytes")
	}
}

func TestNewSchemaInventoryAndForeignKeyValidationHaveDifferentScopes(t *testing.T) {
	database, err := storage.OpenMigration(filepath.Join(t.TempDir(), "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	ctx := context.Background()
	connection, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := storage.LoadSimple(connection); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(ctx, ContentTables); err != nil {
		t.Fatal(err)
	}
	if err := Validate(ctx, connection, 9); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(ctx, "INSERT INTO issues(issue_id,journal_id) VALUES(1,999)"); err != nil {
		t.Fatal(err)
	}
	if err := ValidateStructure(ctx, connection, 9); err != nil {
		t.Fatal("ordinary preflight unexpectedly checks foreign keys", err)
	}
	if err := Validate(ctx, connection, 9); err == nil {
		t.Fatal("explicit verification ignored broken foreign key")
	}
	if _, err := connection.ExecContext(ctx, "DELETE FROM issues; CREATE TABLE unexpected(value TEXT)"); err != nil {
		t.Fatal(err)
	}
	if err := ValidateStructure(ctx, connection, 9); err == nil {
		t.Fatal("unknown table was accepted")
	}
}
