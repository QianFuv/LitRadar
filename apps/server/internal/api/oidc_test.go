package api

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
)

func apiOidcFixture(t *testing.T) (*authHandlers, *http.ServeMux, string, *secrets.Codec) {
	t.Helper()
	handlers, router, token := authFixture(t)
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{4}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	handlers.oidc = auth.NewOidc(handlers.service, settings.New(handlers.repository, codec), codec, false)
	handlers.oidcPool = handlers.pool
	return handlers, router, token, codec
}

func signedOidcCookie(t *testing.T, codec *secrets.Codec, state auth.OidcState) *http.Cookie {
	t.Helper()
	payload, err := json.Marshal(state)
	if err != nil {
		t.Fatal(err)
	}
	signature, err := codec.SignOidcState(payload)
	if err != nil {
		t.Fatal(err)
	}
	return &http.Cookie{Name: oidcCookieName, Value: base64.RawURLEncoding.EncodeToString(payload) + "." + base64.RawURLEncoding.EncodeToString(signature)}
}

// TestOidcCallbackRejectsStateAndProviderErrors clears the flow cookie on every callback failure.
func TestOidcCallbackRejectsStateAndProviderErrors(t *testing.T) {
	for _, test := range []struct{ name, query, target string }{
		{"state", "state=wrong&code=code", "/login?oidc_error=state"},
		{"denied", "state=expected&error=access_denied", "/login?oidc_error=denied"},
		{"provider", "state=expected&error=server_error", "/login?oidc_error=provider"},
		{"duplicate_state", "state=expected&state=other&code=code", "/login?oidc_error=state"},
		{"missing_code", "state=expected", "/login?oidc_error=provider"},
	} {
		t.Run(test.name, func(t *testing.T) {
			_, router, _, codec := apiOidcFixture(t)
			request := httptest.NewRequest("GET", "/api/auth/oidc/callback?"+test.query, nil)
			request.AddCookie(signedOidcCookie(t, codec, auth.OidcState{State: "expected", Nonce: "nonce", Verifier: "verifier", ExpiresAt: time.Now().Add(time.Minute).Unix()}))
			recorder := httptest.NewRecorder()
			router.ServeHTTP(recorder, request)
			if recorder.Code != 302 || recorder.Header().Get("Location") != test.target {
				t.Fatal(recorder.Code, recorder.Header().Get("Location"))
			}
			cookies := recorder.Result().Cookies()
			if len(cookies) != 1 || cookies[0].Name != oidcCookieName || cookies[0].MaxAge != -1 || cookies[0].Path != "/api/auth/oidc" || !cookies[0].HttpOnly || !cookies[0].Secure || cookies[0].SameSite != http.SameSiteLaxMode {
				t.Fatal("callback did not clear secure state cookie")
			}
		})
	}
}

// TestOidcLinkCallbackRequiresLiveLocalSession binds consent to the original user and generation.
func TestOidcLinkCallbackRequiresLiveLocalSession(t *testing.T) {
	_, router, _, codec := apiOidcFixture(t)
	request := httptest.NewRequest("GET", "/api/auth/oidc/callback?state=expected&code=code", nil)
	request.AddCookie(signedOidcCookie(t, codec, auth.OidcState{State: "expected", Nonce: "nonce", Verifier: "verifier", UserId: 1, ExpiresAt: time.Now().Add(time.Minute).Unix()}))
	recorder := httptest.NewRecorder()
	router.ServeHTTP(recorder, request)
	if recorder.Code != 302 || recorder.Header().Get("Location") != "/?oidc_link=expired_session" {
		t.Fatal(recorder.Code, recorder.Header().Get("Location"))
	}
}

// TestOidcPublicEnablementAndRateLimit exposes configuration without credentials and applies IP limits.
func TestOidcPublicEnablementAndRateLimit(t *testing.T) {
	handlers, router, _, _ := apiOidcFixture(t)
	response := authRequest(router, "GET", "/api/auth/providers", "", "")
	if response.Code != 200 || response.Body.String() != `{"oidc_enabled":false,"oidc_display_name":"authentik"}` {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "GET", "/api/auth/oidc/login?next=//evil.example", "", "")
	if response.Code != 302 || response.Header().Get("Location") != "/login?oidc_error=unavailable" {
		t.Fatal(response.Code, response.Header().Get("Location"))
	}
	policy := rateLimitTestPolicy()
	policy.LoginIp.Capacity = 1
	handlers.limiter = newAuthRateLimiter(policy)
	if response = authRequest(router, "GET", "/api/auth/oidc/login", "", ""); response.Code != 302 {
		t.Fatal(response.Code)
	}
	if response = authRequest(router, "GET", "/api/auth/oidc/callback", "", ""); response.Code != 429 {
		t.Fatal("callback bypassed IP limit", response.Code)
	}
	cookies := response.Result().Cookies()
	if len(cookies) != 1 || cookies[0].MaxAge != -1 {
		t.Fatal("rate-limited callback retained state")
	}
}

// TestEmailHttpContract checks null clearing and email sign-in through the real body decoder.
func TestEmailHttpContract(t *testing.T) {
	_, router, token := authFixture(t)
	if response := authRequest(router, "PUT", "/api/auth/email", `{}`, token); response.Code != 422 {
		t.Fatal("missing email was treated as null", response.Code)
	}
	response := authRequest(router, "PUT", "/api/auth/email", `{"email":" Mixed@Example.com "}`, token)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "POST", "/api/auth/login", `{"username":"MIXED@EXAMPLE.COM","password":"correct password long"}`, "")
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "PUT", "/api/auth/email", `{"email":null}`, token)
	if response.Code != 200 {
		t.Fatal(response.Code, response.Body.String())
	}
	response = authRequest(router, "POST", "/api/auth/login", `{"username":"mixed@example.com","password":"correct password long"}`, "")
	if response.Code != 401 {
		t.Fatal(response.Code)
	}
}

// TestEmailRateLimitKeysRetainAddressSuffix avoids collisions between distinct valid addresses.
func TestEmailRateLimitKeysRetainAddressSuffix(t *testing.T) {
	prefix := "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
	first, second := prefix+"@one.example", prefix+"@two.example"
	if normalizeUsername(first) == normalizeUsername(second) || normalizeUsername("A@EXAMPLE.COM") != "a@example.com" {
		t.Fatal("email bucket normalization")
	}
	limiter := newAuthRateLimiter(rateLimitTestPolicy())
	source := directSource("192.0.2.60")
	for range 2 {
		if limiter.checkAt(loginAttempt, source, first, 100) != nil {
			t.Fatal("initial email limit")
		}
	}
	if limiter.checkAt(loginAttempt, source, second, 100) != nil {
		t.Fatal("distinct email shared identifier bucket")
	}
}

// TestOidcStateSignerUsesDeploymentKey proves payload tampering and different keys change the MAC.
func TestOidcStateSignerUsesDeploymentKey(t *testing.T) {
	_, _, _, codec := apiOidcFixture(t)
	first, err := codec.SignOidcState([]byte("state"))
	if err != nil {
		t.Fatal(err)
	}
	second, err := codec.SignOidcState([]byte("other-state"))
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(first, second) || len(first) != sha256.Size {
		t.Fatal("state MAC did not bind payload")
	}
	other, err := secrets.NewCodec(bytes.Repeat([]byte{5}, 32))
	if err != nil {
		t.Fatal(err)
	}
	defer other.Close()
	third, err := other.SignOidcState([]byte("state"))
	if err != nil || bytes.Equal(first, third) {
		t.Fatal("state MAC did not bind deployment key", err)
	}
	codec.Close()
	if _, err := codec.SignOidcState([]byte("state")); err == nil {
		t.Fatal("closed key still signs")
	}
}

// TestOidcOnlyHttpCredentialRefusals applies the same policy to self-service and admin endpoints.
func TestOidcOnlyHttpCredentialRefusals(t *testing.T) {
	handlers, router, admin, user, member := adminFixture(t)
	if err := handlers.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), "UPDATE users SET auth_kind='oidc',password_hash='!oidc' WHERE id=?", user.Id)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct{ method, path, body, token, detail string }{
		{"PUT", "/api/auth/email", `{"email":"a@example.com"}`, member, "Email is managed by the identity provider"},
		{"POST", "/api/auth/change-password", `{"old_password":"old","new_password":"Replacement!2026"}`, member, "Password login is not available for single sign-on accounts"},
		{"POST", "/api/admin/users/" + strconv.FormatInt(int64(user.Id), 10) + "/reset-password", `{"new_password":"Replacement!2026"}`, admin, "Password login is not available for single sign-on accounts"},
	} {
		response := authRequest(router, test.method, test.path, test.body, test.token)
		if response.Code != 400 || !strings.Contains(response.Body.String(), test.detail) {
			t.Fatal(response.Code, response.Body.String())
		}
	}
	response := authRequest(router, "POST", "/api/auth/login", `{"username":"member","password":"long enough password"}`, "")
	if response.Code != 401 || !strings.Contains(response.Body.String(), "Invalid username or password") {
		t.Fatal(response.Code, response.Body.String())
	}
}
