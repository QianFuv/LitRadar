package auth

import (
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"errors"
	"net/url"
	"strings"
	"sync"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
)

func nativeParameters() (NativeOIDCParameters, string) {
	verifier := strings.Repeat("v", 43)
	digest := sha256.Sum256([]byte(verifier))
	return NativeOIDCParameters{RedirectURI: "http://127.0.0.1:40217/oidc/callback", State: "client-state_0123456789", CodeChallenge: base64.RawURLEncoding.EncodeToString(digest[:])}, verifier
}

func nativeGrant(t *testing.T, oidc *Oidc, provider *fakeOidcProvider) (string, domain.User, NativeOIDCParameters, string) {
	t.Helper()
	parameters, verifier := nativeParameters()
	target, cookie, err := oidc.StartNative(t.Context(), parameters)
	if err != nil {
		t.Fatal(err)
	}
	location, err := url.Parse(target)
	if err != nil {
		t.Fatal(err)
	}
	provider.mutex.Lock()
	provider.nonce, provider.challenge = location.Query().Get("nonce"), location.Query().Get("code_challenge")
	provider.mutex.Unlock()
	state, err := oidc.ReadState(cookie)
	if err != nil || state.Native == nil || *state.Native != parameters || state.Verifier == verifier {
		t.Fatal("native parameters or independent server PKCE lost", err)
	}
	code, user, err := oidc.FinishNative(t.Context(), *state, "idp-code", domain.AuditEvent{Action: "oidc_login", Outcome: "completed"})
	if err != nil || len(code) != 43 {
		t.Fatal("native callback failed", err)
	}
	return code, user, parameters, verifier
}

// TestNativeOIDCParameters enforces exact redirect, canonical S256, and bounded client state.
func TestNativeOIDCParameters(t *testing.T) {
	parameters, _ := nativeParameters()
	for _, redirect := range []string{"litradar://oidc/callback", "http://127.0.0.1:1024/oidc/callback", "http://127.0.0.1:65535/oidc/callback"} {
		parameters.RedirectURI = redirect
		if !parameters.Valid() {
			t.Fatal(redirect)
		}
	}
	for _, redirect := range []string{"http://127.0.0.1:1023/oidc/callback", "http://127.0.0.1:65536/oidc/callback", "http://127.0.0.1:04000/oidc/callback", "http://localhost:4000/oidc/callback", "http://[::1]:4000/oidc/callback", "https://127.0.0.1:4000/oidc/callback", "http://user@127.0.0.1:4000/oidc/callback", "http://127.0.0.1:4000/oidc/callback?x=1", "http://127.0.0.1:4000/oidc/callback#x", "litradar://oidc/callback/", "LITRADAR://oidc/callback", "litradar://oidc/%63allback", "litradar://oidc/callback\n"} {
		parameters.RedirectURI = redirect
		if parameters.Valid() || parameters.Redirect("error", "state") != "" {
			t.Fatal("unsafe native redirect admitted", redirect)
		}
	}
	parameters, _ = nativeParameters()
	parameters.State = "client.state~0123456789"
	if !parameters.Valid() {
		t.Fatal("URL-safe state was rejected")
	}
	for _, state := range []string{"short", strings.Repeat("a", 129), "client state 12345678", "client%state12345678", "client-state-12345\n"} {
		parameters.State = state
		if parameters.Valid() {
			t.Fatal("invalid client state admitted", state)
		}
	}
	parameters, _ = nativeParameters()
	for _, challenge := range []string{strings.Repeat("a", 42), strings.Repeat("a", 44), strings.Repeat("a", 43), parameters.CodeChallenge + "=", strings.Repeat("+", 43)} {
		parameters.CodeChallenge = challenge
		if parameters.Valid() {
			t.Fatal("invalid challenge admitted", challenge)
		}
	}
}

// TestNativeOidcKeepsWebSessionAndIssuesDeviceToken covers the fake IdP, quota, TTL and atomic audit.
func TestNativeOidcKeepsWebSessionAndIssuesDeviceToken(t *testing.T) {
	oidc, provider := oidcFixture(t)
	ctx := t.Context()
	web, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil)
	if err != nil {
		t.Fatal(err)
	}
	code, user, parameters, verifier := nativeGrant(t, oidc, provider)
	before, err := oidc.service.ListTokens(ctx, user.ID)
	if err != nil || len(before) != 0 {
		t.Fatal("callback created a personal token", before, err)
	}
	result, err := oidc.ExchangeNative(ctx, code, verifier, parameters.RedirectURI, "  Windows 电脑  ", domain.AuditEvent{Outcome: "completed"})
	if err != nil || result.User.ID != user.ID || result.Token == "" || result.ExpiresAt != oidc.service.now()+90*24*60*60 {
		t.Fatal(result, err)
	}
	if authorization, err := oidc.service.VerifyToken(ctx, web.Token); err != nil || authorization == nil {
		t.Fatal("native sign-in replaced the web session", err)
	}
	tokens, err := oidc.service.ListTokens(ctx, user.ID)
	if err != nil || len(tokens) != 1 || tokens[0].Name != "device:Windows 电脑" {
		t.Fatal(tokens, err)
	}
	events, err := oidc.service.repository.ListAudit(ctx)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, event := range events {
		if event.Action == "oidc_native_exchange" && event.Outcome == "completed" && event.ActorID != nil && *event.ActorID == int64(user.ID) && event.TargetID != nil && *event.TargetID == tokens[0].ID {
			found = true
		}
	}
	if !found {
		t.Fatal("device issuance audit missing")
	}
	if _, err := oidc.ExchangeNative(ctx, code, verifier, parameters.RedirectURI, "Windows", domain.AuditEvent{}); !errors.Is(err, OidcError("invalid_grant")) {
		t.Fatal("code reused", err)
	}
}

// TestNativeOidcInvalidGrants consumes failing grants and rejects stale or removed identities.
func TestNativeOidcInvalidGrants(t *testing.T) {
	for _, scenario := range []string{"pkce", "verifier_format", "redirect", "expiry", "generation", "deleted", "restart"} {
		t.Run(scenario, func(t *testing.T) {
			oidc, provider := oidcFixture(t)
			code, user, parameters, verifier := nativeGrant(t, oidc, provider)
			redirect := parameters.RedirectURI
			switch scenario {
			case "pkce":
				verifier = strings.Repeat("x", 43)
			case "verifier_format":
				verifier = "short"
			case "redirect":
				redirect = "litradar://oidc/callback"
			case "expiry":
				now := oidc.service.now()
				oidc.service.now = func() float64 { return now + 120 }
			case "generation", "deleted":
				if err := oidc.service.repository.Immediate(t.Context(), false, func(connection *sql.Conn) error {
					statement := "UPDATE users SET token_generation=token_generation+1 WHERE id=?"
					if scenario == "deleted" {
						statement = "DELETE FROM users WHERE id=?"
					}
					_, err := connection.ExecContext(t.Context(), statement, user.ID)
					return err
				}); err != nil {
					t.Fatal(err)
				}
			case "restart":
				oidc.nativeCodes = nil
			}
			if _, err := oidc.ExchangeNative(t.Context(), code, verifier, redirect, "Android", domain.AuditEvent{}); !errors.Is(err, OidcError("invalid_grant")) {
				t.Fatal("invalid grant admitted", err)
			}
			_, correctVerifier := nativeParameters()
			if _, err := oidc.ExchangeNative(t.Context(), code, correctVerifier, parameters.RedirectURI, "Android", domain.AuditEvent{}); !errors.Is(err, OidcError("invalid_grant")) {
				t.Fatal("failed attempt did not consume code", err)
			}
		})
	}
}

// TestNativeOidcConcurrentExchange allows one device token across simultaneous code redemptions.
func TestNativeOidcConcurrentExchange(t *testing.T) {
	oidc, provider := oidcFixture(t)
	code, _, parameters, verifier := nativeGrant(t, oidc, provider)
	var workers sync.WaitGroup
	results := make(chan error, 8)
	for range 8 {
		workers.Go(func() {
			_, err := oidc.ExchangeNative(t.Context(), code, verifier, parameters.RedirectURI, "Android", domain.AuditEvent{Outcome: "completed"})
			results <- err
		})
	}
	workers.Wait()
	close(results)
	successes := 0
	for err := range results {
		if err == nil {
			successes++
		} else if !errors.Is(err, OidcError("invalid_grant")) {
			t.Fatal(err)
		}
	}
	if successes != 1 {
		t.Fatal("exchange winners", successes)
	}
}

// TestNativeOidcSharesIdentityPolicy rejects unverified email and requires explicit local-account linking.
func TestNativeOidcSharesIdentityPolicy(t *testing.T) {
	for _, scenario := range []string{"email_unverified", "email_conflict", "nonce"} {
		t.Run(scenario, func(t *testing.T) {
			oidc, provider := oidcFixture(t)
			state := beginOidc(t, oidc, provider, nil)
			parameters, _ := nativeParameters()
			state.Native = &parameters
			expected := scenario
			switch scenario {
			case "email_unverified":
				provider.isVerified = false
			case "email_conflict":
				user, err := oidc.service.Bootstrap(t.Context(), "local_user", "LocalPassword!2026", nil)
				if err != nil {
					t.Fatal(err)
				}
				email := provider.email
				if _, err := oidc.service.UpdateEmail(t.Context(), user.ID, &email, nil); err != nil {
					t.Fatal(err)
				}
			case "nonce":
				provider.hasWrongNonce = true
				expected = "state"
			}
			if _, _, err := oidc.FinishNative(t.Context(), *state, "code", domain.AuditEvent{Outcome: "completed"}); !errors.Is(err, OidcError(expected)) {
				t.Fatal("native identity policy differed from web", err)
			}
		})
	}
}

// TestNativeOidcDeviceValidationAndQuota reuses the password flow's label and personal-token limits.
func TestNativeOidcDeviceValidationAndQuota(t *testing.T) {
	oidc, provider := oidcFixture(t)
	ctx := t.Context()
	for _, name := range []string{"", "  ", "Android\nphone", strings.Repeat("界", 94)} {
		code, _, parameters, verifier := nativeGrant(t, oidc, provider)
		if _, err := oidc.ExchangeNative(ctx, code, verifier, parameters.RedirectURI, name, domain.AuditEvent{}); !errors.Is(err, domain.ErrDeviceName) {
			t.Fatal("invalid native device name admitted", err)
		}
	}
	code, user, parameters, verifier := nativeGrant(t, oidc, provider)
	for range 50 {
		if _, err := oidc.service.CreateTrustedToken(ctx, user.ID, "existing", 3600, nil); err != nil {
			t.Fatal(err)
		}
	}
	if _, err := oidc.ExchangeNative(ctx, code, verifier, parameters.RedirectURI, "Android", domain.AuditEvent{}); !errors.Is(err, domain.ErrTokenLimit) {
		t.Fatal("native OIDC bypassed personal token quota", err)
	}
}
