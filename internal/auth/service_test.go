package auth

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/platform/admission"
	"github.com/QianFuv/LitRadar/internal/platform/cryptography"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

func testService(t *testing.T, useRustFixture bool) *Service {
	t.Helper()
	filename := filepath.Join(t.TempDir(), "auth.sqlite")
	if useRustFixture {
		data, err := os.ReadFile(filepath.Join("..", "..", "tests", "data", "migration", "rust", "auth.sqlite.fixture"))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filename, data, 0600); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := migration.Migrate(context.Background(), filename); err != nil {
		t.Fatal(err)
	}
	repository, err := storage.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { repository.Close() })
	service := New(repository, 2)
	service.now = func() float64 { return 1700000000 }
	return service
}

func TestNewCredentialAndTokenValidationPriority(t *testing.T) {
	service := testService(t, false)
	ctx := context.Background()
	if _, err := service.Bootstrap(ctx, "x", "short", nil); !errors.Is(err, domain.ErrUsername) {
		t.Fatal(err)
	}
	if _, err := service.Bootstrap(ctx, "valid", "密码不足十二个字", nil); !errors.Is(err, domain.ErrPasswordShort) {
		t.Fatal(err)
	}
	for _, name := range []string{"用户名", " valid", "valid ", strings.Repeat("a", 33)} {
		if ValidUsername(name) {
			t.Fatal(name)
		}
	}
	if _, err := ValidateTokenRequest(strings.Repeat(" ", 101), 0); !errors.Is(err, domain.ErrTokenNameLength) {
		t.Fatal(err)
	}
	if _, err := ValidateTokenRequest(" login ", 0); !errors.Is(err, domain.ErrTokenReservedName) {
		t.Fatal(err)
	}
	if _, err := ValidateTokenRequest("valid", 3599); !errors.Is(err, domain.ErrTokenTtl) {
		t.Fatal(err)
	}
	for _, name := range []string{"", "  ", "LOGIN", strings.Repeat("😀", 100)} {
		if _, err := ValidateTokenRequest(name, 3600); err != nil {
			t.Fatal(err)
		}
	}
	user, err := service.Bootstrap(ctx, "unicode", strings.Repeat("😀", 12), nil)
	if err != nil {
		t.Fatal(err)
	}
	if !user.IsAdmin {
		t.Fatal(user)
	}
}

func TestFrozenRustLegacyCredentialsUpgradeWithoutRevokingToken(t *testing.T) {
	service := testService(t, true)
	ctx := context.Background()
	credentials, err := service.repository.CredentialsById(ctx, 1)
	if err != nil || credentials == nil {
		t.Fatal(err)
	}
	if strings.HasPrefix(credentials.PasswordHash, "$argon2") {
		t.Fatal("fixture no longer has legacy credentials")
	}
	preexisting, err := service.CreateTrustedToken(ctx, 1, "kept", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	authorized, err := service.VerifyPasswordAuthorization(ctx, credentials.User.Username, "MigrationFixture!2026")
	if err != nil || authorized == nil {
		t.Fatalf("%v %v", authorized, err)
	}
	current, err := service.repository.CredentialsById(ctx, 1)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(current.PasswordHash, "$argon2id$v=19$m=19456,t=2,p=1$") || current.Salt != "" || current.TokenGeneration != credentials.TokenGeneration {
		t.Fatal("legacy upgrade format/generation changed")
	}
	if found, err := service.VerifyToken(ctx, preexisting.Token); err != nil || found == nil {
		t.Fatalf("existing token lost: %v", err)
	}
	if found, err := service.VerifyPasswordAuthorization(ctx, credentials.User.Username, "wrong"); err != nil || found != nil {
		t.Fatalf("%v %v", found, err)
	}
}

func TestLegacyShortPasswordCanLoginButCannotBeNewPassword(t *testing.T) {
	service := testService(t, false)
	ctx := context.Background()
	if _, err := service.repository.Bootstrap(ctx, "legacy", cryptography.LegacyHash("short", "salt"), "salt", 100, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := service.Login(ctx, "legacy", "short", nil); err != nil {
		t.Fatal(err)
	}
	if _, err := service.ChangePassword(ctx, 1, "short", "short", nil); !errors.Is(err, domain.ErrPasswordShort) {
		t.Fatal(err)
	}
}

func TestLoginReplacementPublicAuthorizationAndGlobalRevocation(t *testing.T) {
	service := testService(t, false)
	ctx := context.Background()
	password := "StrongPassword!2026"
	user, err := service.Bootstrap(ctx, "admin", password, nil)
	if err != nil {
		t.Fatal(err)
	}
	first, err := service.Login(ctx, "ADMIN", password, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(first.Token) != 64 || first.ExpiresAt != 1700604800 {
		t.Fatal("session format or TTL changed")
	}
	authorization, err := service.VerifyToken(ctx, first.Token)
	if err != nil || authorization == nil {
		t.Fatal(err)
	}
	personal, err := service.CreateToken(ctx, *authorization, "  named  ", 3600, nil)
	if err != nil || personal.Name != "named" {
		t.Fatal(err)
	}
	if _, err := service.CreateToken(ctx, domain.Authorization{User: user}, "unsafe", 3600, nil); !errors.Is(err, domain.ErrStaleAuthorization) {
		t.Fatal("public issuance accepted missing bearer")
	}
	if _, err := service.Login(ctx, "admin", password, nil); err != nil {
		t.Fatal(err)
	}
	if found, err := service.VerifyToken(ctx, first.Token); err != nil || found != nil {
		t.Fatal("old login survived replacement")
	}
	if found, err := service.VerifyToken(ctx, personal.Token); err != nil || found == nil {
		t.Fatal("personal token lost on login")
	}
	if _, err := service.CreateToken(ctx, *authorization, "stale", 3600, nil); !errors.Is(err, domain.ErrStaleAuthorization) {
		t.Fatal(err)
	}
	observed, err := service.VerifyPasswordAuthorization(ctx, "admin", password)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := service.RevokeAll(ctx, user.Id, service.event("logout_all", nil)); err != nil {
		t.Fatal(err)
	}
	if _, err := service.createLogin(ctx, *observed, service.event("login", nil)); !errors.Is(err, domain.ErrCredentials) {
		t.Fatal(err)
	}
	if found, err := service.VerifyToken(ctx, personal.Token); err != nil || found != nil {
		t.Fatal("global revocation failed")
	}
}

func TestUnknownUserPerformsDummyPasswordWorkThroughAdmission(t *testing.T) {
	service := testService(t, false)
	service.passwordGate = admission.New(1)
	if cryptography.VerifyPassword("irrelevant", "", dummyPasswordHash) != cryptography.Invalid {
		t.Fatal("dummy credential accepted")
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	entered, release, finished := make(chan struct{}), make(chan struct{}), make(chan struct{})
	go func() {
		defer close(finished)
		admission.Run(context.Background(), service.passwordGate, func() (bool, error) { close(entered); <-release; return true, nil })
	}()
	<-entered
	defer func() { close(release); <-finished }()
	result := make(chan error, 1)
	go func() { _, err := service.VerifyPasswordAuthorization(ctx, "missing", "password"); result <- err }()
	select {
	case err := <-result:
		t.Fatalf("missing-user verification bypassed occupied KDF gate: %v", err)
	case <-time.After(100 * time.Millisecond):
	}
	cancel()
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

func TestLostLegacyUpgradeReverifiesWinningCredentials(t *testing.T) {
	for _, isReset := range []bool{false, true} {
		t.Run(fmt.Sprint(isReset), func(t *testing.T) {
			service := testService(t, true)
			ctx := context.Background()
			password := "MigrationFixture!2026"
			observed, err := service.repository.CredentialsById(ctx, 1)
			if err != nil || observed == nil {
				t.Fatal(err)
			}
			if isReset {
				if _, err := service.ResetPassword(ctx, nil, observed.User.Id, "ConcurrentReset!2026", nil); err != nil {
					t.Fatal(err)
				}
			} else {
				if authorized, err := service.VerifyPasswordAuthorization(ctx, observed.User.Username, password); err != nil || authorized == nil {
					t.Fatal(err)
				}
			}
			authorized, err := service.verifyObservedAuthorization(ctx, observed, password)
			if err != nil {
				t.Fatal(err)
			}
			if isReset && authorized != nil {
				t.Fatal("old password survived a winning reset")
			}
			if !isReset && authorized == nil {
				t.Fatal("equivalent winning upgrade invalidated correct password")
			}
		})
	}
}

func TestPasswordChangeAndResetRevokeOldSessions(t *testing.T) {
	service := testService(t, false)
	ctx := context.Background()
	password := "StrongPassword!2026"
	user, err := service.Bootstrap(ctx, "admin", password, nil)
	if err != nil {
		t.Fatal(err)
	}
	session, err := service.Login(ctx, "admin", password, nil)
	if err != nil {
		t.Fatal(err)
	}
	if changed, err := service.ChangePassword(ctx, user.Id, "wrong", "Replacement!2026", nil); err != nil || changed {
		t.Fatalf("%t %v", changed, err)
	}
	if changed, err := service.ChangePassword(ctx, user.Id, password, "Replacement!2026", nil); err != nil || !changed {
		t.Fatalf("%t %v", changed, err)
	}
	if found, err := service.VerifyToken(ctx, session.Token); err != nil || found != nil {
		t.Fatal("password rotation retained token")
	}
	if _, err := service.Login(ctx, "admin", password, nil); !errors.Is(err, domain.ErrCredentials) {
		t.Fatal(err)
	}
	if _, err := service.Login(ctx, "admin", "Replacement!2026", nil); err != nil {
		t.Fatal(err)
	}
	if changed, err := service.ResetPassword(ctx, &user.Id, user.Id, "Administrative!2026", nil); err != nil || !changed {
		t.Fatalf("%t %v", changed, err)
	}
	if _, err := service.Login(ctx, "admin", "Administrative!2026", nil); err != nil {
		t.Fatal(err)
	}
}

func TestInviteLifecycleProjectionAndOneTimeRegistration(t *testing.T) {
	service := testService(t, false)
	ctx := context.Background()
	user, err := service.Bootstrap(ctx, "admin", "StrongPassword!2026", nil)
	if err != nil {
		t.Fatal(err)
	}
	invite, err := service.IssueInvite(ctx, user.Id, false, nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(invite.Code) != 16 || invite.Status != "active" || invite.MaxUses != 1 || invite.RevokedAt != nil {
		t.Fatal("invite policy changed")
	}
	if _, err := service.Register(ctx, "member", "StrongPassword!2026", &invite.Code, nil); err != nil {
		t.Fatal(err)
	}
	used, err := service.Invite(ctx, user.Id)
	if err != nil || used.Status != "exhausted" || !used.Used {
		t.Fatalf("%v %v", used, err)
	}
	if _, err := service.IssueInvite(ctx, user.Id, false, nil); !errors.Is(err, domain.ErrActiveInvite) {
		t.Fatal(err)
	}
	replacement, err := service.IssueInvite(ctx, user.Id, true, nil)
	if err != nil || replacement.Code == invite.Code {
		t.Fatal(err)
	}
	if revoked, err := service.RevokeInvite(ctx, user.Id, nil); err != nil || !revoked {
		t.Fatalf("%t %v", revoked, err)
	}
	latest, err := service.Invite(ctx, user.Id)
	if err != nil || latest.Status != "revoked" {
		t.Fatalf("%v %v", latest, err)
	}
}

func TestPrivateJsonAndAllLoggingPathsRedactSecrets(t *testing.T) {
	secret := "DO_NOT_LOG_SYNTHETIC_SECRET"
	values := []any{domain.User{Id: identity.Id(9007199254740993), Username: secret}, domain.Credentials{PasswordHash: secret, Salt: secret}, domain.Authorization{TokenHash: &secret}, domain.IssuedToken{Token: secret, Name: secret}, domain.LoginSession{Token: secret}, domain.InviteRow{Code: secret}, domain.InviteResponse{Code: secret}}
	for _, value := range values {
		if formatted := fmt.Sprintf("%v %+v %#v", value, value, value); strings.Contains(formatted, secret) {
			t.Fatalf("format leaked %T", value)
		}
		for _, isJson := range []bool{false, true} {
			var buffer bytes.Buffer
			var handler slog.Handler = slog.NewTextHandler(&buffer, nil)
			if isJson {
				handler = slog.NewJSONHandler(&buffer, nil)
			}
			slog.New(handler).Info("synthetic", slog.Any("value", value))
			if strings.Contains(buffer.String(), secret) {
				t.Fatalf("log leaked %T json=%t", value, isJson)
			}
		}
	}
	for _, value := range []any{domain.Credentials{PasswordHash: secret}, domain.Authorization{TokenHash: &secret}, domain.LoginSession{Token: secret}, domain.InviteRow{Code: secret}} {
		encoded, err := json.Marshal(value)
		if err != nil || strings.Contains(string(encoded), secret) {
			t.Fatalf("private JSON leaked %T: %v", value, err)
		}
	}
	encoded, err := json.Marshal(domain.IssuedToken{Token: secret})
	if err != nil || !strings.Contains(string(encoded), secret) {
		t.Fatal("authorized one-time response lost token")
	}
	encoded, err = json.Marshal(domain.User{Id: identity.Id(9007199254740993)})
	if err != nil || !strings.Contains(string(encoded), `"id":"9007199254740993"`) {
		t.Fatal("identifier precision lost")
	}
}
