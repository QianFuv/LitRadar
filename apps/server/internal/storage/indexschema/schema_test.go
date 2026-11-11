package indexschema

import (
	"context"
	"database/sql"

	"path/filepath"
	"testing"

	storage "github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlite"
)

func TestNewSchemaInventoryAndForeignKeyValidationHaveDifferentScopes(t *testing.T) {
	database, err := storage.OpenMigration(filepath.Join(t.TempDir(), "content.sqlite"))
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := database.Close(); err != nil {
			t.Errorf("database.Close: %v", err)
		}
	}()
	ctx := context.Background()
	connection, err := database.Conn(ctx)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := connection.Close(); err != nil {
			t.Errorf("connection.Close: %v", err)
		}
	}()
	if _, err := connection.ExecContext(ctx, ContentTables); err != nil {
		t.Fatal(err)
	}
	if err := Validate(ctx, connection, Version); err != nil {
		t.Fatal(err)
	}
	if _, err := connection.ExecContext(ctx, "INSERT INTO issues(issue_id,journal_id) VALUES(1,999)"); err != nil {
		t.Fatal(err)
	}
	if err := ValidateStructure(ctx, connection, Version); err != nil {
		t.Fatal("ordinary preflight unexpectedly checks foreign keys", err)
	}
	if err := Validate(ctx, connection, Version); err == nil {
		t.Fatal("explicit verification ignored broken foreign key")
	}
	assertUnexpectedSchemaTableRejected(t, connection)
}

func assertUnexpectedSchemaTableRejected(t *testing.T, connection *sql.Conn) {
	t.Helper()
	ctx := context.Background()
	if _, err := connection.ExecContext(ctx, "DELETE FROM issues; CREATE TABLE unexpected(value TEXT)"); err != nil {
		t.Fatal(err)
	}
	if err := ValidateStructure(ctx, connection, Version); err == nil {
		t.Fatal("unknown table was accepted")
	}
}
