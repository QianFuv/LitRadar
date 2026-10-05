package auth

import (
	"context"
	"testing"
)

func TestTokenHashRetainsOriginalPythonCompatibleDigest(t *testing.T) {
	if actual := HashToken("token"); actual != "3c469e9d6c5875d37a43f353d4f88e61fcf812c66eee3457465a40b0da4153e0" {
		t.Fatal(actual)
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
