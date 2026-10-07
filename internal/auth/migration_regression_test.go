package auth

import (
	"context"
	"database/sql"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/platform/cryptography"
)

func TestTokenHashRetainsOriginalPythonCompatibleDigest(t *testing.T) {
	if actual := HashToken("token"); actual != "3c469e9d6c5875d37a43f353d4f88e61fcf812c66eee3457465a40b0da4153e0" {
		t.Fatal(actual)
	}
}

// observedLegacyCredentials captures a legacy row before another credential update wins.
func observedLegacyCredentials(t *testing.T, service *Service, password string) *domain.Credentials {
	t.Helper()
	ctx := context.Background()
	if _, err := service.repository.Bootstrap(ctx, "legacy", cryptography.LegacyHash(password, "salt"), "salt", 100, nil); err != nil {
		t.Fatal(err)
	}
	observed, err := service.repository.CredentialsById(ctx, 1)
	if err != nil || observed == nil {
		t.Fatal(observed, err)
	}
	return observed
}

// TestLostLegacyReloadRetainsCapturedUserAndFreshGeneration preserves captured identity projection.
func TestLostLegacyReloadRetainsCapturedUserAndFreshGeneration(t *testing.T) {
	service := testService(t)
	ctx := context.Background()
	password := "SyntheticLegacy!2026"
	observed := observedLegacyCredentials(t, service, password)
	if authorization, err := service.VerifyPasswordAuthorization(ctx, "legacy", password); err != nil || authorization == nil {
		t.Fatal(authorization, err)
	}
	if err := service.repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "UPDATE users SET username='updated',is_admin=0,token_generation=7 WHERE id=1")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	authorization, err := service.verifyObservedAuthorization(ctx, observed, password)
	if err != nil || authorization == nil {
		t.Fatal(authorization, err)
	}
	if authorization.User != observed.User || authorization.TokenGeneration != 7 || authorization.TokenHash != nil {
		t.Fatal("winning credential reload changed captured identity or lost its fresh generation")
	}
}

// TestLostLegacyReloadDeniesDeletedUser covers a missing row after credential work.
func TestLostLegacyReloadDeniesDeletedUser(t *testing.T) {
	service := testService(t)
	ctx := context.Background()
	password := "SyntheticLegacy!2026"
	observed := observedLegacyCredentials(t, service, password)
	if err := service.repository.Immediate(ctx, false, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "DELETE FROM users WHERE id=1")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if authorization, err := service.verifyObservedAuthorization(ctx, observed, password); err != nil || authorization != nil {
		t.Fatal(authorization, err)
	}
}

func TestAbsentUserCompletesRealDummyVerificationWithoutCreatingIdentity(t *testing.T) {
	service := testService(t)
	if authorization, err := service.VerifyPasswordAuthorization(context.Background(), "missing", "password"); err != nil || authorization != nil {
		t.Fatal(authorization, err)
	}
	if credentials, err := service.repository.CredentialsByName(context.Background(), "missing"); err != nil || credentials != nil {
		t.Fatal(credentials, err)
	}
	if events, err := service.repository.ListAudit(context.Background()); err != nil || len(events) != 0 {
		t.Fatal(events, err)
	}
}
