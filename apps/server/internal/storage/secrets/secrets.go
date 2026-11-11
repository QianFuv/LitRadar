// Package secrets performs explicit transactional maintenance of persisted integration credentials.
package secrets

import (
	"context"
	"database/sql"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cryptography"
	platformsqlite "github.com/QianFuv/LitRadar/apps/server/internal/platform/sqlite"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/sqlc/secretsdb"
)

var ErrAuthentication = errors.New("Stored secret authentication failed")
var ErrLegacyPlaintext = errors.New("Legacy plaintext secret found; run `admin secrets migrate` before startup")

// Codec adapts primitive errors to the persisted-secret application's stable diagnostics.
type Codec struct{ primitive *cryptography.Codec }

// NewCodec copies an exact raw key into the primitive's directly owned buffer.
func NewCodec(key []byte) (*Codec, error) {
	if len(key) != 32 {
		return nil, fmt.Errorf("Secret key file must contain exactly 32 bytes; found %d", len(key))
	}
	primitive, err := cryptography.NewCodec(key)
	if err != nil {
		return nil, err
	}
	return &Codec{primitive}, nil
}

func (codec *Codec) String() string       { return "SecretCodec([REDACTED])" }
func (codec *Codec) GoString() string     { return codec.String() }
func (codec *Codec) LogValue() slog.Value { return slog.StringValue(codec.String()) }

// Close clears only the directly owned key buffer; it must follow completion of all concurrent users.
func (codec *Codec) Close() { codec.primitive.Close() }

// Encrypt preserves the frozen envelope and row/field associated-data format.
func (codec *Codec) Encrypt(plaintext, associated string) (string, error) {
	value, err := codec.primitive.Encrypt(plaintext, associated)
	if err != nil {
		return "", ErrAuthentication
	}
	return value, nil
}

// Decrypt authenticates before returning UTF-8 plaintext and preserves legacy diagnostics.
func (codec *Codec) Decrypt(stored, associated string) (string, error) {
	if strings.HasPrefix(stored, "litradarenc:v1:") && strings.ContainsAny(stored, "\r\n") {
		return "", ErrAuthentication
	}
	value, err := codec.primitive.Decrypt(stored, associated)
	if errors.Is(err, cryptography.ErrLegacyPlaintext) {
		return "", ErrLegacyPlaintext
	}
	if err != nil {
		return "", ErrAuthentication
	}
	return value, nil
}

// Load reads exactly 32 raw deployment-key bytes without creating or replacing the file.
func Load(filename string) (*Codec, error) {
	key, err := os.ReadFile(filename)
	if err != nil {
		return nil, fmt.Errorf("Unable to read the secret key file: %w", err)
	}
	defer clear(key)
	return NewCodec(key)
}

// NotificationContext binds a secret to its owner and notification column.
func NotificationContext(user int64, field string) string {
	return fmt.Sprintf("notification_settings:%d:%s", user, field)
}

// RuntimeContext binds a secret to one managed runtime field.
func RuntimeContext(field string) string { return "runtime_settings:" + field }

// CnkiContext binds the complete encrypted session state to its user.
func CnkiContext(user int64) string { return fmt.Sprintf("cnki_sessions:%d:session_json", user) }

// PoolReferenceContext separates removable pool references from persisted field ciphertext.
func PoolReferenceContext(field string) string { return RuntimeContext(field) + ":pool-item-reference" }

// IsRuntimeSecret identifies encrypted registry fields; unknown rows remain untouched.
func IsRuntimeSecret(field string) bool {
	switch field {
	case "openalex_api_key_pool", "semantic_scholar_api_key_pool", "cnki_captcha_token", "provider_proxy_url", "oidc_client_secret":
		return true
	default:
		return false
	}
}

// MigrationReport counts encrypted, authenticated and cleared values in a committed migration.
type MigrationReport struct {
	Migrated int
	Verified int
	Empty    int
}

// VerificationReport counts authenticated and cleared values without promising a read snapshot.
type VerificationReport struct {
	Verified int
	Empty    int
}

// Migrate explicitly encrypts plaintext and authenticates any existing version-prefixed envelope.
func Migrate(ctx context.Context, filename string, codec *Codec) (MigrationReport, error) {
	var report MigrationReport
	err := visit(ctx, filename, true, func(stored, associated string) (*string, error) {
		if stored == "" {
			report.Empty++
			return nil, nil
		}
		if strings.HasPrefix(stored, "litradarenc:") {
			if _, err := codec.Decrypt(stored, associated); err != nil {
				return nil, err
			}
			report.Verified++
			return nil, nil
		}
		encrypted, err := codec.Encrypt(stored, associated)
		if err != nil {
			return nil, err
		}
		report.Migrated++
		return &encrypted, nil
	})
	if err != nil {
		return MigrationReport{}, err
	}
	return report, nil
}

// Verify authenticates all nonempty secrets without migrating plaintext or opening a read transaction.
func Verify(ctx context.Context, filename string, codec *Codec) (VerificationReport, error) {
	var report VerificationReport
	err := visit(ctx, filename, false, func(stored, associated string) (*string, error) {
		if stored == "" {
			report.Empty++
			return nil, nil
		}
		if _, err := codec.Decrypt(stored, associated); err != nil {
			return nil, err
		}
		report.Verified++
		return nil, nil
	})
	if err != nil {
		return VerificationReport{}, err
	}
	return report, nil
}

// Rotate reencrypts all stored nonempty values atomically and never changes either key file.
func Rotate(ctx context.Context, filename string, oldCodec, newCodec *Codec) (int, error) {
	rotated := 0
	err := visit(ctx, filename, true, func(stored, associated string) (*string, error) {
		if stored == "" {
			return nil, nil
		}
		plaintext, err := oldCodec.Decrypt(stored, associated)
		if err != nil {
			return nil, err
		}
		encrypted, err := newCodec.Encrypt(plaintext, associated)
		if err != nil {
			return nil, err
		}
		rotated++
		return &encrypted, nil
	})
	if err != nil {
		return 0, err
	}
	return rotated, nil
}

type secretValue struct {
	stored, associated, statement string
	identity                      any
}

var storageQueries secretsdb.Queries

// visit owns connection and optional transaction lifetimes across ordered table reads.
func visit(ctx context.Context, filename string, isMutation bool, visitor func(string, string) (*string, error)) error {
	database, err := platformsqlite.Open(platformsqlite.Config{Filename: filename, Mode: "rwc", MaxConnections: 1})
	if err != nil {
		return err
	}
	defer database.Close()
	connection, err := database.Conn(ctx)
	if err != nil {
		return err
	}
	defer connection.Close()
	if isMutation {
		if _, err := connection.ExecContext(ctx, "BEGIN IMMEDIATE"); err != nil {
			return err
		}
		defer connection.ExecContext(context.Background(), "ROLLBACK")
	}
	for _, query := range []func(context.Context, *sql.Conn) ([]secretValue, error){notificationValues, runtimeValues, cnkiValues} {
		values, err := query(ctx, connection)
		if err != nil {
			return err
		}
		if err := visitSecretValues(ctx, connection, values, visitor); err != nil {
			return err
		}
	}
	if isMutation {
		_, err = connection.ExecContext(ctx, "COMMIT")
		return err
	}
	return nil
}

// visitSecretValues applies callbacks and optional replacements after a complete table read.
func visitSecretValues(ctx context.Context, connection *sql.Conn, values []secretValue, visitor func(string, string) (*string, error)) error {
	for _, value := range values {
		replacement, err := visitor(value.stored, value.associated)
		if err != nil {
			return err
		}
		if replacement != nil {
			if _, err := connection.ExecContext(ctx, value.statement, *replacement, value.identity); err != nil {
				return err
			}
		}
	}
	return nil
}

func notificationValues(ctx context.Context, connection *sql.Conn) ([]secretValue, error) {
	rows, err := storageQueries.Notifications(ctx, connection)
	if err != nil {
		return nil, err
	}
	result := make([]secretValue, 0, len(rows)*3)
	for _, row := range rows {
		fields := []struct{ name, stored, statement string }{{"pushplus_token", string(row.PushplusToken), secretsdb.UpdatePushplus}, {"ai_api_key", string(row.AIAPIKey), secretsdb.UpdatePrimaryAI}, {"ai_backup_api_key", string(row.AIBackupAPIKey), secretsdb.UpdateBackupAI}}
		for _, field := range fields {
			result = append(result, secretValue{field.stored, NotificationContext(row.UserID, field.name), field.statement, row.UserID})
		}
	}
	return result, nil
}

func runtimeValues(ctx context.Context, connection *sql.Conn) ([]secretValue, error) {
	rows, err := storageQueries.RuntimeSettings(ctx, connection)
	if err != nil {
		return nil, err
	}
	result := []secretValue{}
	for _, row := range rows {
		field := string(row.Key)
		if IsRuntimeSecret(field) {
			result = append(result, secretValue{string(row.Value), RuntimeContext(field), secretsdb.UpdateRuntime, field})
		}
	}
	return result, nil
}

func cnkiValues(ctx context.Context, connection *sql.Conn) ([]secretValue, error) {
	rows, err := storageQueries.CNKISessions(ctx, connection)
	if err != nil {
		return nil, err
	}
	result := make([]secretValue, 0, len(rows))
	for _, row := range rows {
		result = append(result, secretValue{string(row.SessionJSON), CnkiContext(row.UserID), secretsdb.UpdateCNKI, row.UserID})
	}
	return result, nil
}

// SignOidcState authenticates transient browser flow state without disclosing the deployment key.
func (codec *Codec) SignOidcState(payload []byte) ([]byte, error) {
	return codec.primitive.SignOidcState(payload)
}
