package auth

import (
	"context"
	"errors"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
)

// TestEmailValidationAndLocalLogin covers normalized unique email and retained username access.
func TestEmailValidationAndLocalLogin(t *testing.T) {
	for _, value := range []string{"a", "@host", "local@", "a@@b", "a b@c", "a@b\u0000", strings.Repeat("a", 250) + "@host", "a@b\u2003c"} {
		if _, err := NormalizeEmail(&value); !errors.Is(err, domain.ErrEmail) {
			t.Fatalf("invalid email accepted: %q", value)
		}
	}
	for _, value := range []string{"", "  "} {
		if address, err := NormalizeEmail(&value); address != nil || err != nil {
			t.Fatalf("blank email not treated as absent: %q", value)
		}
	}
	service := testService(t)
	ctx := context.Background()
	first, err := service.Bootstrap(ctx, "first_user", "StrongPassword!2026", nil)
	if err != nil {
		t.Fatal(err)
	}
	address := " Mixed.Case@Example.com "
	user, err := service.UpdateEmail(ctx, first.Id, &address, nil)
	if err != nil || user.Email == nil || *user.Email != "mixed.case@example.com" {
		t.Fatal("email normalization", err)
	}
	for _, identifier := range []string{"first_user", "MIXED.CASE@EXAMPLE.COM"} {
		session, err := service.Login(ctx, identifier, "StrongPassword!2026", nil)
		if err != nil || session.User.Id != first.Id {
			t.Fatal("identifier lookup failed", err)
		}
	}
	invite, err := service.IssueInvite(ctx, first.Id, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	invalidInvite := "not-a-valid-invite"
	if _, err := service.Register(ctx, "second_user", "StrongPassword!2026", &invalidInvite, nil, &address); !errors.Is(err, domain.ErrInvite) {
		t.Fatal("registered email disclosed before invite redemption", err)
	}
	if _, err := service.Register(ctx, "second_user", "StrongPassword!2026", &invite.Code, nil, &address); !errors.Is(err, domain.ErrEmailExists) {
		t.Fatal("duplicate registration email accepted", err)
	}
	secondAddress := "second@example.com"
	second, err := service.Register(ctx, "second_user", "StrongPassword!2026", &invite.Code, nil, &secondAddress)
	if err != nil || second.Email == nil {
		t.Fatal("registration consumed invite on rejected email", err)
	}
	if _, err := service.UpdateEmail(ctx, second.Id, &address, nil); !errors.Is(err, domain.ErrEmailExists) {
		t.Fatal("duplicate email update accepted", err)
	}
	if _, err := service.UpdateEmail(ctx, first.Id, nil, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := service.Login(ctx, "mixed.case@example.com", "StrongPassword!2026", nil); !errors.Is(err, domain.ErrCredentials) {
		t.Fatal("cleared email still authenticates", err)
	}
	if _, err := service.UpdateEmail(ctx, second.Id, &address, nil); err != nil {
		t.Fatal("cleared email remained reserved", err)
	}
}
