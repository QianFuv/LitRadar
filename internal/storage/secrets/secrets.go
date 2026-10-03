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
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/platform/cryptography"
	platformsqlite "github.com/QianFuv/LitRadar/internal/platform/sqlite"
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

// IsRuntimeSecret is the persisted registry's four secret fields; unknown rows remain untouched.
func IsRuntimeSecret(field string) bool {
	switch field {
	case "openalex_api_key_pool", "semantic_scholar_api_key_pool", "cnki_captcha_token", "provider_proxy_url":
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

type strictText string

func (value *strictText) Scan(source any) error {
	decoded, ok := source.(string)
	if !ok || !utf8.ValidString(decoded) {
		return errors.New("invalid secret storage text column")
	}
	*value = strictText(decoded)
	return nil
}

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
	}
	if isMutation {
		_, err = connection.ExecContext(ctx, "COMMIT")
		return err
	}
	return nil
}

func notificationValues(ctx context.Context, connection *sql.Conn) ([]secretValue, error) {
	rows, err := connection.QueryContext(ctx, "SELECT user_id,pushplus_token,ai_api_key,ai_backup_api_key FROM notification_settings")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []secretValue{}
	for rows.Next() {
		var user int64
		var values [3]strictText
		if err := rows.Scan(&user, &values[0], &values[1], &values[2]); err != nil {
			return nil, err
		}
		for index, field := range []string{"pushplus_token", "ai_api_key", "ai_backup_api_key"} {
			result = append(result, secretValue{string(values[index]), NotificationContext(user, field), "UPDATE notification_settings SET " + field + "=? WHERE user_id=?", user})
		}
	}
	return result, rows.Err()
}

func runtimeValues(ctx context.Context, connection *sql.Conn) ([]secretValue, error) {
	rows, err := connection.QueryContext(ctx, "SELECT key,value FROM runtime_settings")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []secretValue{}
	for rows.Next() {
		var key, value strictText
		if err := rows.Scan(&key, &value); err != nil {
			return nil, err
		}
		field := string(key)
		if IsRuntimeSecret(field) {
			result = append(result, secretValue{string(value), RuntimeContext(field), "UPDATE runtime_settings SET value=? WHERE key=?", field})
		}
	}
	return result, rows.Err()
}

func cnkiValues(ctx context.Context, connection *sql.Conn) ([]secretValue, error) {
	rows, err := connection.QueryContext(ctx, "SELECT user_id,session_json FROM cnki_sessions")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []secretValue{}
	for rows.Next() {
		var user int64
		var value strictText
		if err := rows.Scan(&user, &value); err != nil {
			return nil, err
		}
		result = append(result, secretValue{string(value), CnkiContext(user), "UPDATE cnki_sessions SET session_json=? WHERE user_id=?", user})
	}
	return result, rows.Err()
}
