package auth

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
)

// TestNativeLoginFencesPasswordAuthority checks revocation between password verification and issuance.
func TestNativeLoginFencesPasswordAuthority(t *testing.T) {
	service := testService(t)
	ctx := context.Background()
	if _, err := service.Bootstrap(ctx, "native_admin", "correct password long", nil); err != nil {
		t.Fatal(err)
	}
	didRevoke := false
	service.now = func() float64 {
		if !didRevoke {
			didRevoke = true
			if _, err := service.repository.RevokeAll(ctx, 1, domain.AuditEvent{Action: "logout_all", Outcome: "completed", OccurredAt: 1700000000}); err != nil {
				t.Fatal(err)
			}
		}
		return 1700000000
	}
	if _, err := service.NativeLogin(ctx, "native_admin", "correct password long", "Windows", nil); !errors.Is(err, domain.ErrCredentials) {
		t.Fatal("stale password authority issued a device token", err)
	}
	if tokens, err := service.repository.ListTokens(ctx, 1, 1700000000); err != nil || len(tokens) != 0 {
		t.Fatal(tokens, err)
	}
}

// TestNativeLoginHashesTokensAndRollsBackAuditFailures checks the existing durable token machinery.
func TestNativeLoginHashesTokensAndRollsBackAuditFailures(t *testing.T) {
	service := testService(t)
	ctx := context.Background()
	if _, err := service.Bootstrap(ctx, "native_admin", "correct password long", nil); err != nil {
		t.Fatal(err)
	}
	issued, err := service.NativeLogin(ctx, "native_admin", "correct password long", strings.Repeat("界", 93), nil)
	if err != nil || issued.ExpiresAt != 1700000000+90*24*60*60 {
		t.Fatal(issued, err)
	}
	if err := service.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		var hash string
		if err := connection.QueryRowContext(ctx, "SELECT token_hash FROM access_tokens WHERE name!= 'login'").Scan(&hash); err != nil {
			return err
		}
		if hash != HashToken(issued.Token) || hash == issued.Token {
			t.Fatal("raw device token persisted")
		}
		_, err := connection.ExecContext(ctx, "CREATE TRIGGER reject_native_audit BEFORE INSERT ON security_audit_events WHEN NEW.action='native_login' BEGIN SELECT RAISE(ABORT,'fixture'); END")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := service.NativeLogin(ctx, "native_admin", "correct password long", "Android", nil); !errors.Is(err, domain.ErrAudit) {
		t.Fatal(err)
	}
	tokens, err := service.ListTokens(ctx, 1)
	if err != nil || len(tokens) != 1 {
		t.Fatal("failed audit left a token", tokens, err)
	}
}
