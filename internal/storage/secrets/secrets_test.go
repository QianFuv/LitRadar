package secrets

import (
	"bytes"
	"context"
	"database/sql"

	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"

	platformsqlite "github.com/QianFuv/LitRadar/internal/platform/sqlite"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func secretDatabase(t *testing.T) (string, *sql.DB) {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	database, err := platformsqlite.Open(platformsqlite.Config{Filename: filename, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { database.Close() })
	statement := `INSERT INTO users(id,username,password_hash,salt,created_at,updated_at) VALUES(1,'synthetic','hash','salt',1,1);
INSERT INTO notification_settings(user_id,pushplus_token,ai_api_key,ai_backup_api_key,created_at,updated_at) VALUES(1,'notify-token','ai-key','',1,1);
INSERT INTO runtime_settings(key,value,updated_at) VALUES('provider_proxy_url','http://synthetic:credential@127.0.0.1:8080',1),('unknown_secret_like_field','untouched',1),('crossref_mailto_pool','contact@example.invalid',1);
INSERT INTO cnki_sessions(user_id,session_json,status,created_at,updated_at) VALUES(1,'{"bff_user_token":"synthetic-token"}','active',1,1);`
	if _, err := database.Exec(statement); err != nil {
		t.Fatal(err)
	}
	return filename, database
}

func testCodec(t *testing.T, value byte) *Codec {
	t.Helper()
	codec, err := NewCodec(bytes.Repeat([]byte{value}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	return codec
}

// TestThreeTableMigrateVerifyAndRotatePreserveMetadata checks three-table migration, authentication and key replacement.
func TestThreeTableMigrateVerifyAndRotatePreserveMetadata(t *testing.T) {
	filename, database := secretDatabase(t)
	codec := testCodec(t, 42)
	replacement := testCodec(t, 43)
	ctx := context.Background()
	assertThreeTableMigrationAndVerification(t, ctx, filename, codec)
	if count, err := Rotate(ctx, filename, codec, replacement); err != nil || count != 4 {
		t.Fatalf("%d %v", count, err)
	}
	if _, err := Verify(ctx, filename, codec); !errors.Is(err, ErrAuthentication) {
		t.Fatal("old deployment key still valid")
	}
	if _, err := Verify(ctx, filename, replacement); err != nil {
		t.Fatal(err)
	}
	assertSecretMaintenanceMetadata(t, database)
}

func TestEnvelopeRejectsLineBreaksInEncodedFields(t *testing.T) {
	codec := testCodec(t, 42)
	stored, err := codec.Encrypt("synthetic", CnkiContext(1))
	if err != nil {
		t.Fatal(err)
	}
	for _, field := range []int{2, 3} {
		for _, newline := range []string{"\r", "\n"} {
			parts := strings.Split(stored, ":")
			parts[field] = parts[field][:1] + newline + parts[field][1:]
			if _, err := codec.Decrypt(strings.Join(parts, ":"), CnkiContext(1)); !errors.Is(err, ErrAuthentication) {
				t.Errorf("encoded field %d accepted line break %q: %v", field, newline, err)
			}
		}
	}
}

func TestMaintenanceRejectsBlobAndInvalidTextWithoutPartialEncryption(t *testing.T) {
	for _, statement := range []string{
		"UPDATE notification_settings SET ai_api_key=CAST('synthetic' AS BLOB)",
		"UPDATE runtime_settings SET value=CAST('synthetic' AS BLOB) WHERE key='unknown_secret_like_field'",
		"UPDATE cnki_sessions SET session_json=CAST('synthetic' AS BLOB)",
		"UPDATE cnki_sessions SET session_json=CAST(x'ff' AS TEXT)",
	} {
		t.Run(statement, func(t *testing.T) {
			filename, database := secretDatabase(t)
			codec := testCodec(t, 42)
			if _, err := database.Exec(statement); err != nil {
				t.Fatal(err)
			}
			if _, err := Migrate(context.Background(), filename, codec); err == nil {
				t.Fatal("malformed SQLite value was encrypted")
			}
			var value string
			if err := database.QueryRow("SELECT pushplus_token FROM notification_settings").Scan(&value); err != nil || value != "notify-token" {
				t.Fatalf("partial migration escaped rollback: %q %v", value, err)
			}
			if _, err := database.Exec("UPDATE notification_settings SET pushplus_token='',ai_api_key='',ai_backup_api_key=''; UPDATE runtime_settings SET value='' WHERE key='provider_proxy_url'"); err != nil {
				t.Fatal(err)
			}
			if _, err := database.Exec(statement); err != nil {
				t.Fatal(err)
			}
			if _, err := Verify(context.Background(), filename, codec); err == nil {
				t.Fatal("verification accepted malformed SQLite value")
			}
		})
	}
}

// TestMalformedLaterValueRollsBackEarlierSecretChanges checks late-table failures preserve earlier plaintext and exact ciphertext.
func TestMalformedLaterValueRollsBackEarlierSecretChanges(t *testing.T) {
	filename, database := secretDatabase(t)
	codec := testCodec(t, 42)
	ctx := context.Background()
	assertUnknownSecretEnvelopeMigrationRollsBack(t, ctx, filename, database, codec)
	if _, err := database.Exec("UPDATE cnki_sessions SET session_json='{}'"); err != nil {
		t.Fatal(err)
	}
	if _, err := Migrate(ctx, filename, codec); err != nil {
		t.Fatal(err)
	}
	var value string
	if err := database.QueryRow("SELECT ai_api_key FROM notification_settings").Scan(&value); err != nil {
		t.Fatal(err)
	}
	before := value
	if _, err := database.Exec("UPDATE cnki_sessions SET session_json='litradarenc:v1:broken'"); err != nil {
		t.Fatal(err)
	}
	if _, err := Rotate(ctx, filename, codec, testCodec(t, 43)); !errors.Is(err, ErrAuthentication) {
		t.Fatal(err)
	}
	if err := database.QueryRow("SELECT ai_api_key FROM notification_settings").Scan(&value); err != nil || value != before {
		t.Fatal("earlier rotation escaped rollback")
	}
}

func TestKeyFileIsExactBinaryAndNeverReplaced(t *testing.T) {
	filename := filepath.Join(t.TempDir(), "key")
	if _, err := Load(filename); err == nil {
		t.Fatal("missing key created")
	}
	for _, size := range []int{0, 31, 33, 64} {
		if err := os.WriteFile(filename, bytes.Repeat([]byte{42}, size), 0600); err != nil {
			t.Fatal(err)
		}
		if _, err := Load(filename); err == nil {
			t.Fatalf("accepted key size %d", size)
		}
	}
	key := bytes.Repeat([]byte{42}, 32)
	if err := os.WriteFile(filename, key, 0600); err != nil {
		t.Fatal(err)
	}
	codec, err := Load(filename)
	if err != nil {
		t.Fatal(err)
	}
	codec.Close()
	after, err := os.ReadFile(filename)
	if err != nil || !bytes.Equal(after, key) {
		t.Fatal("key file changed")
	}
}

func TestPoolReferencesCannotCrossFieldsOrCiphertextRoles(t *testing.T) {
	codec := testCodec(t, 42)
	reference, err := codec.Encrypt("synthetic-key", PoolReferenceContext("openalex_api_key_pool"))
	if err != nil {
		t.Fatal(err)
	}
	for _, context := range []string{PoolReferenceContext("semantic_scholar_api_key_pool"), RuntimeContext("openalex_api_key_pool")} {
		if _, err := codec.Decrypt(reference, context); !errors.Is(err, ErrAuthentication) {
			t.Fatal(err)
		}
	}
	if plain, err := codec.Decrypt(reference, PoolReferenceContext("openalex_api_key_pool")); err != nil || plain != "synthetic-key" {
		t.Fatal(err)
	}
}

// assertThreeTableMigrationAndVerification checks plaintext rejection, encryption counts and repeated authentication counts.
func assertThreeTableMigrationAndVerification(t *testing.T, ctx context.Context, filename string, codec *Codec) {
	t.Helper()
	if _, err := Verify(ctx, filename, codec); !errors.Is(err, ErrLegacyPlaintext) {
		t.Fatal(err)
	}
	report, err := Migrate(ctx, filename, codec)
	if err != nil || report != (MigrationReport{Migrated: 4, Empty: 1}) {
		t.Fatalf("%+v %v", report, err)
	}
	verified, err := Verify(ctx, filename, codec)
	if err != nil || verified != (VerificationReport{Verified: 4, Empty: 1}) {
		t.Fatalf("%+v %v", verified, err)
	}
	report, err = Migrate(ctx, filename, codec)
	if err != nil || report != (MigrationReport{Verified: 4, Empty: 1}) {
		t.Fatalf("%+v %v", report, err)
	}
}

// assertSecretMaintenanceMetadata checks unknown fields and selected-secret timestamps survive maintenance.
func assertSecretMaintenanceMetadata(t *testing.T, database *sql.DB) {
	t.Helper()
	var value string
	var updated float64
	if err := database.QueryRow("SELECT value,updated_at FROM runtime_settings WHERE key='unknown_secret_like_field'").Scan(&value, &updated); err != nil || value != "untouched" || updated != 1 {
		t.Fatalf("%s %v", value, err)
	}
	if err := database.QueryRow("SELECT value,updated_at FROM runtime_settings WHERE key='provider_proxy_url'").Scan(&value, &updated); err != nil || !strings.HasPrefix(value, "litradarenc:v1:") || updated != 1 {
		t.Fatal(err)
	}
}

// assertUnknownSecretEnvelopeMigrationRollsBack checks a later unknown envelope rolls back earlier plaintext encryption.
func assertUnknownSecretEnvelopeMigrationRollsBack(t *testing.T, ctx context.Context, filename string, database *sql.DB, codec *Codec) {
	t.Helper()
	if _, err := database.Exec("UPDATE cnki_sessions SET session_json='litradarenc:v999:broken'"); err != nil {
		t.Fatal(err)
	}
	if _, err := Migrate(ctx, filename, codec); err == nil {
		t.Fatal("unknown envelope was encrypted twice")
	}
	var value string
	if err := database.QueryRow("SELECT ai_api_key FROM notification_settings").Scan(&value); err != nil || value != "ai-key" {
		t.Fatal("earlier encryption escaped rollback")
	}
}
