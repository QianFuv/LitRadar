package auth

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"errors"
	"maps"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/domain/identity"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/admission"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/cryptography"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/testkit/oidctest"
)

type fakeOidcProvider struct {
	key                                        *oidctest.Key
	hasInvalidSignature                        bool
	mutex                                      sync.Mutex
	issuer, nonce, challenge                   string
	subject, email, preferredUsername          string
	isVerified, hasWrongNonce, hasWrongSubject bool
	tokenRequests, discoveries                 int
	claims                                     map[string]any
}

func oidcFixture(t *testing.T) (*Oidc, *fakeOidcProvider) {
	t.Helper()
	service := testService(t)
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{9}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	provider := &fakeOidcProvider{key: oidctest.NewKey(t), subject: "provider-user-uuid", email: "Verified@Example.com", isVerified: true}
	server := httptest.NewTLSServer(http.HandlerFunc(provider.serve))
	t.Cleanup(server.Close)
	provider.issuer = server.URL
	repository := settings.New(service.repository, codec)
	values := map[string]*string{}
	for field, value := range map[string]string{"oidc_issuer_url": server.URL, "oidc_client_id": "client", "oidc_client_secret": "secret", "public_base_url": "https://paper.example"} {
		values[field] = &value
	}
	if _, err := repository.Update(t.Context(), nil, values, nil, nil); err != nil {
		t.Fatal(err)
	}
	oidc := NewOidc(service, repository, codec, false)
	oidc.client = server.Client()
	oidc.client.Timeout = 5 * time.Second
	oidc.client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	return oidc, provider
}

func (provider *fakeOidcProvider) serve(writer http.ResponseWriter, request *http.Request) {
	provider.mutex.Lock()
	defer provider.mutex.Unlock()
	writer.Header().Set("Content-Type", "application/json")
	var payload any
	switch request.URL.Path {
	case "/.well-known/openid-configuration":
		provider.discoveries++
		payload = map[string]any{"issuer": provider.issuer, "authorization_endpoint": provider.issuer + "/authorize", "token_endpoint": provider.issuer + "/token", "userinfo_endpoint": provider.issuer + "/userinfo", "jwks_uri": provider.issuer + "/jwks"}
	case "/jwks":
		payload = provider.key.JWKS()
	case "/token":
		provider.tokenRequests++
		if err := request.ParseForm(); err != nil {
			writer.WriteHeader(400)
			return
		}
		client, secret, ok := request.BasicAuth()
		digest := sha256.Sum256([]byte(request.Form.Get("code_verifier")))
		if !ok || client != "client" || secret != "secret" || request.Form.Get("grant_type") != "authorization_code" || request.Form.Get("code") == "" || request.Form.Get("redirect_uri") != "https://paper.example/api/auth/oidc/callback" || base64.RawURLEncoding.EncodeToString(digest[:]) != provider.challenge {
			writer.WriteHeader(400)
			return
		}
		nonce := provider.nonce
		if provider.hasWrongNonce {
			nonce = "wrong"
		}
		claims := map[string]any{"iss": provider.issuer, "sub": provider.subject, "aud": "client", "exp": time.Now().Add(time.Hour).Unix(), "iat": time.Now().Unix(), "nonce": nonce}
		maps.Copy(claims, provider.claims)
		encoded, err := provider.key.Sign(claims)
		if err != nil {
			writer.WriteHeader(500)
			return
		}
		if provider.hasInvalidSignature {
			parts := strings.Split(encoded, ".")
			parts[2] = base64.RawURLEncoding.EncodeToString(make([]byte, 256))
			encoded = strings.Join(parts, ".")
		}
		payload = map[string]any{"access_token": "fake-access-token", "token_type": "Bearer", "id_token": encoded}
	case "/userinfo":
		if request.Header.Get("Authorization") != "Bearer fake-access-token" {
			writer.WriteHeader(401)
			return
		}
		subject := provider.subject
		if provider.hasWrongSubject {
			subject = "another-subject"
		}
		profile := map[string]any{"sub": subject, "email": provider.email, "email_verified": provider.isVerified}
		if provider.preferredUsername != "" {
			profile["preferred_username"] = provider.preferredUsername
		}
		payload = profile
	default:
		writer.WriteHeader(404)
		return
	}
	if err := json.NewEncoder(writer).Encode(payload); err != nil {
		return
	}
}

func beginOidc(t *testing.T, oidc *Oidc, provider *fakeOidcProvider, link *domain.Authorization) *OidcState {
	t.Helper()
	target, cookie, err := oidc.Start(t.Context(), "/account?tab=security", link)
	if err != nil {
		t.Fatal(err)
	}
	location, err := url.Parse(target)
	if err != nil {
		t.Fatal(err)
	}
	query := location.Query()
	if query.Get("code_challenge_method") != "S256" || query.Get("response_type") != "code" || query.Get("scope") != "openid profile email" {
		t.Fatal("incorrect authorization request")
	}
	provider.mutex.Lock()
	provider.nonce, provider.challenge = query.Get("nonce"), query.Get("code_challenge")
	provider.mutex.Unlock()
	state, err := oidc.ReadState(cookie)
	if err != nil || !MatchOidcState(state.State, query.Get("state")) {
		t.Fatal("invalid signed state", err)
	}
	return state
}

func finishOidc(t *testing.T, oidc *Oidc, state *OidcState, link *domain.Authorization) (domain.LoginSession, error) {
	t.Helper()
	return oidc.Finish(t.Context(), *state, "fake-code", link, domain.AuditEvent{Action: "oidc_login", Outcome: "completed", OccurredAt: oidc.service.now()})
}

// TestOidcProvisionReturningLoginAndPasswordRefusals covers trusted TLS exchange and OIDC-only credentials.
func TestOidcProvisionReturningLoginAndPasswordRefusals(t *testing.T) {
	oidc, provider := oidcFixture(t)
	state := beginOidc(t, oidc, provider, nil)
	first, err := finishOidc(t, oidc, state, nil)
	if err != nil {
		t.Fatal(err)
	}
	if first.User.AuthKind != "oidc" || first.User.IsAdmin || !first.User.IsOidcLinked || first.User.Email == nil || *first.User.Email != "verified@example.com" || first.Token == "" {
		t.Fatal("invalid provisioned projection")
	}
	credentials, err := oidc.service.repository.CredentialsByID(t.Context(), first.User.ID)
	if err != nil {
		t.Fatal(err)
	}
	if credentials.PasswordHash != domain.UnusablePasswordHash || cryptography.VerifyPassword("anything", "", credentials.PasswordHash) != cryptography.Invalid {
		t.Fatal("OIDC password is usable")
	}
	if _, err := oidc.service.Login(t.Context(), *first.User.Email, "anything", nil); !errors.Is(err, domain.ErrCredentials) {
		t.Fatal(err)
	}
	if _, err := oidc.service.ChangePassword(t.Context(), first.User.ID, "anything", "Replacement!2026", nil); !errors.Is(err, domain.ErrOidcPassword) {
		t.Fatal(err)
	}
	for _, actor := range []*identity.ID{nil, &first.User.ID} {
		if _, err := oidc.service.ResetPassword(t.Context(), actor, first.User.ID, "Replacement!2026", nil); !errors.Is(err, domain.ErrOidcPassword) {
			t.Fatal(err)
		}
	}
	if _, err := oidc.service.UpdateEmail(t.Context(), first.User.ID, nil, nil); !errors.Is(err, domain.ErrManagedEmail) {
		t.Fatal(err)
	}
	provider.mutex.Lock()
	provider.email = "new@example.com"
	provider.mutex.Unlock()
	second, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil)
	if err != nil {
		t.Fatal(err)
	}
	if second.User.ID != first.User.ID || *second.User.Email != "new@example.com" {
		t.Fatal("returning identity changed or email did not sync")
	}
	if old, err := oidc.service.VerifyToken(t.Context(), first.Token); err != nil || old != nil {
		t.Fatal("old login session survived replacement", err)
	}
	provider.mutex.Lock()
	defer provider.mutex.Unlock()
	if provider.discoveries != 1 || provider.tokenRequests != 2 {
		t.Fatal("discovery cache or exchange count", provider.discoveries, provider.tokenRequests)
	}
}

// TestOidcProvisionUsesPreferredUsername keeps a valid free authentik username and falls back otherwise.
func TestOidcProvisionUsesPreferredUsername(t *testing.T) {
	oidc, provider := oidcFixture(t)
	if _, err := oidc.service.Bootstrap(t.Context(), "taken_name", "LocalPassword!2026", nil); err != nil {
		t.Fatal(err)
	}
	cases := []struct{ subject, email, preferred, expected string }{
		{"subject-fresh", "fresh@example.com", "Fresh_Name", "Fresh_Name"},
		{"subject-taken", "taken@example.com", "TAKEN_NAME", ""},
		{"subject-invalid", "invalid@example.com", "invalid.name", ""},
	}
	for _, item := range cases {
		provider.mutex.Lock()
		provider.subject, provider.email, provider.preferredUsername = item.subject, item.email, item.preferred
		provider.mutex.Unlock()
		session, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil)
		if err != nil {
			t.Fatal(item.preferred, err)
		}
		if item.expected != "" && session.User.Username != item.expected {
			t.Fatal("preferred username not used", session.User.Username)
		}
		if item.expected == "" && (!strings.HasPrefix(session.User.Username, "oidc_") || !ValidUsername(session.User.Username)) {
			t.Fatal("fallback username not generated", session.User.Username)
		}
	}
}

// TestOidcClaimAndProfileRejections checks each trust boundary before any identity is persisted.
func TestOidcClaimAndProfileRejections(t *testing.T) {
	cases := []struct {
		name, code string
		configure  func(*fakeOidcProvider)
	}{
		{"signature", "provider", func(provider *fakeOidcProvider) { provider.hasInvalidSignature = true }},
		{"not_before", "provider", func(provider *fakeOidcProvider) {
			provider.claims = map[string]any{"nbf": time.Now().Add(time.Hour).Unix()}
		}},
		{"nonce", "state", func(provider *fakeOidcProvider) { provider.hasWrongNonce = true }},
		{"email_unverified", "email_unverified", func(provider *fakeOidcProvider) { provider.isVerified = false }},
		{"userinfo_subject", "provider", func(provider *fakeOidcProvider) { provider.hasWrongSubject = true }},
		{"issuer", "provider", func(provider *fakeOidcProvider) { provider.claims = map[string]any{"iss": "https://other.example"} }},
		{"audience", "provider", func(provider *fakeOidcProvider) { provider.claims = map[string]any{"aud": "other-client"} }},
		{"multi_audience_without_azp", "provider", func(provider *fakeOidcProvider) { provider.claims = map[string]any{"aud": []string{"client", "other"}} }},
		{"azp", "provider", func(provider *fakeOidcProvider) { provider.claims = map[string]any{"azp": "other"} }},
		{"expired", "provider", func(provider *fakeOidcProvider) {
			provider.claims = map[string]any{"exp": time.Now().Add(-time.Minute).Unix()}
		}},
		{"future_iat", "provider", func(provider *fakeOidcProvider) {
			provider.claims = map[string]any{"iat": time.Now().Add(time.Hour).Unix()}
		}},
		{"missing_iat", "provider", func(provider *fakeOidcProvider) { provider.claims = map[string]any{"iat": 0} }},
	}
	for _, test := range cases {
		t.Run(test.name, func(t *testing.T) {
			oidc, provider := oidcFixture(t)
			provider.mutex.Lock()
			test.configure(provider)
			provider.mutex.Unlock()
			_, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil)
			if err == nil || err.Error() != test.code {
				t.Fatalf("got %v, want %s", err, test.code)
			}
			required, err := oidc.service.BootstrapRequired(t.Context())
			if err != nil || !required {
				t.Fatal("rejected identity persisted", err)
			}
		})
	}
}

// TestOidcEmailConflictAndExplicitLinking never links an existing local account by email.
func TestOidcEmailConflictAndExplicitLinking(t *testing.T) {
	oidc, provider := oidcFixture(t)
	ctx := t.Context()
	local, err := oidc.service.Bootstrap(ctx, "local_user", "LocalPassword!2026", nil)
	if err != nil {
		t.Fatal(err)
	}
	email := "verified@example.com"
	if _, err := oidc.service.UpdateEmail(ctx, local.ID, &email, nil); err != nil {
		t.Fatal(err)
	}
	if _, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil); err == nil || err.Error() != "email_conflict" {
		t.Fatal(err)
	}
	login, err := oidc.service.Login(ctx, local.Username, "LocalPassword!2026", nil)
	if err != nil {
		t.Fatal(err)
	}
	authorization, err := oidc.service.VerifyToken(ctx, login.Token)
	if err != nil {
		t.Fatal(err)
	}
	linked, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, authorization), authorization)
	if err != nil {
		t.Fatal(err)
	}
	if linked.User.ID != local.ID || linked.User.AuthKind != "local" || !linked.User.IsOidcLinked || linked.Token != "" {
		t.Fatal("link changed local credentials")
	}
	returned, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil)
	if err != nil || returned.User.ID != local.ID {
		t.Fatal("linked sign-in failed", err)
	}
	authorization, err = oidc.service.VerifyToken(ctx, returned.Token)
	if err != nil {
		t.Fatal(err)
	}
	unlinked, err := oidc.Unlink(ctx, *authorization, domain.AuditEvent{Outcome: "completed", OccurredAt: oidc.service.now()})
	if err != nil || unlinked.IsOidcLinked {
		t.Fatal("unlink failed", err)
	}
	if _, err := oidc.service.Login(ctx, local.Username, "LocalPassword!2026", nil); err != nil {
		t.Fatal("password access lost after unlink", err)
	}
}

// TestOidcLinkConflictAndRevokedSession refuses identity reuse and generation changes during consent.
func TestOidcLinkConflictAndRevokedSession(t *testing.T) {
	oidc, provider := oidcFixture(t)
	ctx := t.Context()
	if _, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil); err != nil {
		t.Fatal(err)
	}
	if err := oidc.service.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "INSERT INTO users(username,password_hash,salt,is_admin,created_at,updated_at) VALUES('local_user','hash','',1,1,1)")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	credentials, err := oidc.service.repository.CredentialsByName(ctx, "local_user")
	if err != nil {
		t.Fatal(err)
	}
	token, err := oidc.service.CreateTrustedToken(ctx, credentials.User.ID, "test", 3600, nil)
	if err != nil {
		t.Fatal(err)
	}
	authorization, err := oidc.service.VerifyToken(ctx, token.Token)
	if err != nil {
		t.Fatal(err)
	}
	state := beginOidc(t, oidc, provider, authorization)
	if _, err := finishOidc(t, oidc, state, authorization); err == nil || err.Error() != "account_conflict" {
		t.Fatal(err)
	}
	provider.mutex.Lock()
	provider.subject = "unused-subject"
	provider.mutex.Unlock()
	state = beginOidc(t, oidc, provider, authorization)
	if _, err := oidc.service.RevokeAll(ctx, credentials.User.ID, domain.AuditEvent{Action: "logout_all", Outcome: "completed", OccurredAt: oidc.service.now()}); err != nil {
		t.Fatal(err)
	}
	if _, err := finishOidc(t, oidc, state, authorization); err == nil || err.Error() != "expired_session" {
		t.Fatal(err)
	}
}

// TestOidcStateAndRedirectSafety rejects tampering, expiration and off-site redirect targets.
func TestOidcStateAndRedirectSafety(t *testing.T) {
	oidc, provider := oidcFixture(t)
	_, cookie, err := oidc.Start(t.Context(), "/safe", nil)
	if err != nil {
		t.Fatal(err)
	}
	for _, encoded := range []string{"", cookie + "x", strings.Replace(cookie, ".", "x.", 1)} {
		if _, err := oidc.ReadState(encoded); err == nil {
			t.Fatal("tampered state accepted")
		}
	}
	state := beginOidc(t, oidc, provider, nil)
	if MatchOidcState(state.State, "mismatch") || MatchOidcState(state.State, "") {
		t.Fatal("state mismatch accepted")
	}
	for _, next := range []string{"https://evil.example", "//evil.example", "/%2f/evil.example", "/\\evil.example", "/%5cevil.example", "/ok\r\nLocation: x"} {
		target, cookie, err := oidc.Start(t.Context(), next, nil)
		if err != nil || target == "" {
			t.Fatal(err)
		}
		state, err := oidc.ReadState(cookie)
		if err != nil || state.Next != "/" {
			t.Fatal("unsafe redirect accepted", next, err)
		}
	}
	state.ExpiresAt = time.Now().Add(-time.Second).Unix()
	if _, err := finishOidc(t, oidc, state, nil); err == nil || err.Error() != "state" {
		t.Fatal(err)
	}
}

// TestOidcPasswordUsesDummyAdmission requires the same bounded KDF work as an unknown identifier.
func TestOidcPasswordUsesDummyAdmission(t *testing.T) {
	oidc, provider := oidcFixture(t)
	session, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil)
	if err != nil {
		t.Fatal(err)
	}
	gate := admission.New(1)
	oidc.service.passwordGate = gate
	entered, release, finished := make(chan struct{}), make(chan struct{}), make(chan struct{})
	go func() {
		defer close(finished)
		_, err := admission.Run(t.Context(), gate, func() (bool, error) { close(entered); <-release; return true, nil })
		if err != nil {
			return
		}
	}()
	<-entered
	defer func() { close(release); <-finished }()
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	result := make(chan error, 1)
	go func() { _, err := oidc.service.Login(ctx, session.User.Username, "password", nil); result <- err }()
	select {
	case err := <-result:
		t.Fatal("OIDC account bypassed KDF gate", err)
	case <-time.After(50 * time.Millisecond):
	}
	cancel()
	if err := <-result; !errors.Is(err, context.Canceled) {
		t.Fatal(err)
	}
}

// TestOidcDiscoveryCacheExpires bounds provider metadata staleness.
func TestOidcDiscoveryCacheExpires(t *testing.T) {
	oidc, provider := oidcFixture(t)
	beginOidc(t, oidc, provider, nil)
	oidc.mutex.Lock()
	oidc.cacheUntil = time.Now().Add(-time.Second)
	oidc.mutex.Unlock()
	beginOidc(t, oidc, provider, nil)
	provider.mutex.Lock()
	defer provider.mutex.Unlock()
	if provider.discoveries != 2 {
		t.Fatal("expired discovery was reused")
	}
}

// TestOidcProvisionAuditFailureRollsBackIdentity keeps failed security mutations atomic.
func TestOidcProvisionAuditFailureRollsBackIdentity(t *testing.T) {
	oidc, provider := oidcFixture(t)
	ctx := t.Context()
	if err := oidc.service.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		_, err := connection.ExecContext(ctx, "CREATE TRIGGER reject_oidc_audit BEFORE INSERT ON security_audit_events WHEN NEW.action='oidc_provision' BEGIN SELECT RAISE(ABORT,'blocked'); END")
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if _, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil); !errors.Is(err, domain.ErrAudit) {
		t.Fatal("audit failure was ignored", err)
	}
	required, err := oidc.service.BootstrapRequired(ctx)
	if err != nil || !required {
		t.Fatal("failed provisioning persisted account", err)
	}
	if err := oidc.service.repository.WithConnection(ctx, func(connection *sql.Conn) error {
		var count int
		if err := connection.QueryRowContext(ctx, "SELECT count(*) FROM user_identities").Scan(&count); err != nil {
			return err
		}
		if count != 0 {
			return errors.New("failed provisioning persisted identity")
		}
		return nil
	}); err != nil {
		t.Fatal(err)
	}
}

// TestOidcSigningKeyRotation verifies that a new kid refreshes the cached provider's JWKS.
func TestOidcSigningKeyRotation(t *testing.T) {
	oidc, provider := oidcFixture(t)
	if _, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil); err != nil {
		t.Fatal(err)
	}
	key := oidctest.NewKey(t)
	provider.mutex.Lock()
	provider.key = key
	provider.mutex.Unlock()
	if _, err := finishOidc(t, oidc, beginOidc(t, oidc, provider, nil), nil); err != nil {
		t.Fatal("rotated key rejected", err)
	}
}
