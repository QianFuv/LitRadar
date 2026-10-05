package indexschema

import (
	"context"

	"path/filepath"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

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
