package nativehost

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"net/url"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"
)

// Server-side validation patterns of the native OIDC contract.
var (
	serverStatePattern    = regexp.MustCompile(`^[A-Za-z0-9._~-]{16,128}$`)
	serverVerifierPattern = regexp.MustCompile(`^[A-Za-z0-9._~-]{43,128}$`)
)

// fakeOidcServer implements the server half of the native OIDC contract.
type fakeOidcServer struct {
	t           *testing.T
	code        string
	mutex       sync.Mutex
	challenge   string
	redirectURI string
	exchanges   []map[string]string
	rejectAll   bool
}

// ServeHTTP answers the exchange endpoint; the browser step runs in openURL.
func (server *fakeOidcServer) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	if request.Method != http.MethodPost || request.URL.Path != nativeOidcExchangePath || request.Header.Get("Authorization") != "" {
		http.NotFound(writer, request)
		return
	}
	var body map[string]string
	_ = json.NewDecoder(request.Body).Decode(&body)
	server.mutex.Lock()
	server.exchanges = append(server.exchanges, body)
	challenge, redirectURI := server.challenge, server.redirectURI
	server.mutex.Unlock()
	digest := sha256.Sum256([]byte(body["code_verifier"]))
	isValid := !server.rejectAll && body["code"] == server.code &&
		serverVerifierPattern.MatchString(body["code_verifier"]) &&
		base64.RawURLEncoding.EncodeToString(digest[:]) == challenge &&
		body["redirect_uri"] == redirectURI && body["device_name"] == "Windows test-host"
	writer.Header().Set("Content-Type", "application/json")
	if !isValid {
		writer.WriteHeader(http.StatusBadRequest)
		_, _ = writer.Write([]byte(`{"detail":"invalid_grant","code":"bad_request","retryable":false}`))
		return
	}
	_, _ = writer.Write([]byte(`{"token":"sso-token","expires_at":1900000000,"user":{"id":9,"username":"sso_member"}}`))
}

// browserRedirect returns an OpenURL hook that plays the browser: it checks
// the login URL, records the PKCE challenge, and follows the server's final
// redirect to the callback with the given query fields.
func (server *fakeOidcServer) browserRedirect(origin string, callbackQuery func(state string) url.Values) func(string) error {
	return func(rawURL string) error {
		loginURL, err := url.Parse(rawURL)
		if err != nil || !strings.HasPrefix(rawURL, origin+nativeOidcLoginPath+"?") {
			server.t.Errorf("unexpected login URL %q", rawURL)
			return errors.New("unexpected login URL")
		}
		query := loginURL.Query()
		state, challenge, redirectURI := query.Get("state"), query.Get("code_challenge"), query.Get("redirect_uri")
		if !serverStatePattern.MatchString(state) || len(challenge) != 43 {
			server.t.Errorf("login URL parameters state=%q challenge=%q", state, challenge)
		}
		server.mutex.Lock()
		server.challenge, server.redirectURI = challenge, redirectURI
		server.mutex.Unlock()
		go func() {
			response, err := http.Get(redirectURI + "?" + callbackQuery(state).Encode())
			if err == nil {
				_ = response.Body.Close()
			}
		}()
		return nil
	}
}

// exchangeCount returns how many exchanges the server received.
func (server *fakeOidcServer) exchangeCount() int {
	server.mutex.Lock()
	defer server.mutex.Unlock()
	return len(server.exchanges)
}

// newOidcTestBridge starts the fake server and a loopback bridge whose browser
// returns callbackQuery.
func newOidcTestBridge(t *testing.T, callbackQuery func(state string) url.Values) (*Bridge, *fakeOidcServer, *memorySecretStore, string) {
	t.Helper()
	fake := &fakeOidcServer{t: t, code: strings.Repeat("c", 43)}
	server := httptest.NewTLSServer(fake)
	t.Cleanup(server.Close)
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)
	bridge.options.OidcCallbacks = LoopbackOidcCallbacks{}
	bridge.options.OpenURL = fake.browserRedirect(origin, callbackQuery)
	return bridge, fake, secrets, origin
}

func TestNewOidcProofMatchesTheServerContract(t *testing.T) {
	first, second := newOidcProof(), newOidcProof()
	for _, proof := range []oidcProof{first, second} {
		if !serverVerifierPattern.MatchString(proof.verifier) || !serverStatePattern.MatchString(proof.state) {
			t.Fatalf("proof %+v does not match the server patterns", proof)
		}
		digest := sha256.Sum256([]byte(proof.verifier))
		if proof.challenge != base64.RawURLEncoding.EncodeToString(digest[:]) || len(proof.challenge) != 43 {
			t.Fatalf("challenge %q is not the S256 of the verifier", proof.challenge)
		}
	}
	if first.verifier == second.verifier || first.state == second.state || first.state == first.verifier {
		t.Fatal("proofs repeat random values")
	}
}

func TestReadOidcCallback(t *testing.T) {
	validCode := strings.Repeat("A", 43)
	tests := []struct {
		name       string
		query      url.Values
		wantCode   string
		wantReason string
	}{
		{"code", url.Values{"code": {validCode}, "state": {"s"}}, validCode, ""},
		{"server error", url.Values{"error": {"denied"}, "state": {"s"}}, "", "denied"},
		{"error wins over code", url.Values{"error": {"email_conflict"}, "code": {validCode}}, "", "email_conflict"},
		{"unbounded error", url.Values{"error": {"<script>"}}, "", "provider"},
		{"repeated error", url.Values{"error": {"denied", "state"}}, "", "provider"},
		{"missing code", url.Values{"state": {"s"}}, "", "invalid_callback"},
		{"repeated code", url.Values{"code": {validCode, validCode}}, "", "invalid_callback"},
		{"malformed code", url.Values{"code": {"short"}}, "", "invalid_callback"},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			code, reason := readOidcCallback(test.query)
			if code != test.wantCode || reason != test.wantReason {
				t.Fatalf("readOidcCallback = %q, %q; want %q, %q", code, reason, test.wantCode, test.wantReason)
			}
		})
	}
}

func TestLoopbackListenerAcceptsOnlyTheMatchingCallbackOnce(t *testing.T) {
	listener, err := LoopbackOidcCallbacks{}.Listen("expected-state-0123456789")
	if err != nil {
		t.Fatalf("Listen: %v", err)
	}
	defer listener.Close()
	redirectURI := listener.RedirectURI()
	if !regexp.MustCompile(`^http://127\.0\.0\.1:[0-9]{4,5}/oidc/callback$`).MatchString(redirectURI) {
		t.Fatalf("redirect URI = %q", redirectURI)
	}
	base := strings.TrimSuffix(redirectURI, OidcCallbackPath)
	expectStatus := func(method, target string, want int) {
		t.Helper()
		request, _ := http.NewRequest(method, target, nil)
		response, err := http.DefaultClient.Do(request)
		if err != nil {
			t.Fatalf("%s %s: %v", method, target, err)
		}
		_ = response.Body.Close()
		if response.StatusCode != want {
			t.Fatalf("%s %s = %d, want %d", method, target, response.StatusCode, want)
		}
	}
	expectStatus(http.MethodGet, base+"/other?state=expected-state-0123456789&code=x", http.StatusNotFound)
	expectStatus(http.MethodPost, redirectURI+"?state=expected-state-0123456789&code=x", http.StatusNotFound)
	expectStatus(http.MethodGet, redirectURI+"?state=wrong-state-0123456789&code=x", http.StatusBadRequest)
	expectStatus(http.MethodGet, redirectURI+"?code=x", http.StatusBadRequest)

	waitContext, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
	if _, err := listener.Wait(waitContext); !errors.Is(err, context.DeadlineExceeded) {
		t.Fatalf("Wait after rejected callbacks = %v, want deadline", err)
	}
	cancel()

	expectStatus(http.MethodGet, redirectURI+"?state=expected-state-0123456789&code=first", http.StatusOK)
	expectStatus(http.MethodGet, redirectURI+"?state=expected-state-0123456789&code=second", http.StatusGone)
	query, err := listener.Wait(context.Background())
	if err != nil || query.Get("code") != "first" {
		t.Fatalf("Wait = %v, %v", query, err)
	}

	listener.Close()
	listener.Close()
	if response, err := http.Get(redirectURI); err == nil {
		_ = response.Body.Close()
		t.Fatal("listener still answers after Close")
	}
}

func TestLoopbackListenerRejectsForeignHostHeaders(t *testing.T) {
	listener, err := LoopbackOidcCallbacks{}.Listen("expected-state-0123456789")
	if err != nil {
		t.Fatalf("Listen: %v", err)
	}
	defer listener.Close()
	request, _ := http.NewRequest(http.MethodGet, listener.RedirectURI()+"?state=expected-state-0123456789&code=x", nil)
	request.Host = "attacker.example"
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		t.Fatalf("request: %v", err)
	}
	_ = response.Body.Close()
	if response.StatusCode != http.StatusNotFound {
		t.Fatalf("status = %d, want 404", response.StatusCode)
	}
}

func TestLoginWithOidcExchangesTheCodeAndStoresTheToken(t *testing.T) {
	bridge, fake, secrets, origin := newOidcTestBridge(t, func(state string) url.Values {
		return url.Values{"code": {strings.Repeat("c", 43)}, "state": {state}}
	})
	isFocused := false
	bridge.options.FocusApp = func() { isFocused = true }

	response, err := bridge.LoginWithOidc(context.Background())
	if err != nil {
		t.Fatalf("LoginWithOidc: %v", err)
	}
	if response.Status != http.StatusOK || strings.Contains(string(response.Body), "sso-token") ||
		!strings.Contains(string(response.Body), `"sso_member"`) {
		t.Fatalf("response = %d %s", response.Status, response.Body)
	}
	if token, _, _ := secrets.Get(tokenKey(origin)); token != "sso-token" {
		t.Fatalf("stored token = %q", token)
	}
	if fake.exchangeCount() != 1 || !isFocused {
		t.Fatalf("exchanges = %d, focused = %v", fake.exchangeCount(), isFocused)
	}
	if response, err := http.Get(fake.redirectURI); err == nil {
		_ = response.Body.Close()
		t.Fatal("loopback listener still open after sign-in")
	}
}

func TestLoginWithOidcReportsCallbackErrorsWithoutExchanging(t *testing.T) {
	bridge, fake, secrets, origin := newOidcTestBridge(t, func(state string) url.Values {
		return url.Values{"error": {"denied"}, "state": {state}}
	})

	_, err := bridge.LoginWithOidc(context.Background())
	var bridgeError *BridgeError
	if !errors.As(err, &bridgeError) || bridgeError.Code != CodeOidcError || bridgeError.Reason != "denied" {
		t.Fatalf("LoginWithOidc error = %v", err)
	}
	if fake.exchangeCount() != 0 {
		t.Fatal("a failed callback was exchanged")
	}
	if _, hasToken, _ := secrets.Get(tokenKey(origin)); hasToken {
		t.Fatal("a token was stored")
	}
}

func TestLoginWithOidcReturnsRejectedExchanges(t *testing.T) {
	bridge, fake, secrets, origin := newOidcTestBridge(t, func(state string) url.Values {
		return url.Values{"code": {strings.Repeat("c", 43)}, "state": {state}}
	})
	fake.rejectAll = true

	response, err := bridge.LoginWithOidc(context.Background())
	if err != nil || response.Status != http.StatusBadRequest || !strings.Contains(string(response.Body), "invalid_grant") {
		t.Fatalf("LoginWithOidc = %d %s, %v", response.Status, response.Body, err)
	}
	if _, hasToken, _ := secrets.Get(tokenKey(origin)); hasToken {
		t.Fatal("a token was stored after a rejected exchange")
	}
}

func TestLoginWithOidcStopsWhenCancelledOrTimedOut(t *testing.T) {
	bridge, fake, _, _ := newOidcTestBridge(t, nil)
	opened := make(chan string, 2)
	bridge.options.OpenURL = func(rawURL string) error {
		loginURL, _ := url.Parse(rawURL)
		opened <- loginURL.Query().Get("redirect_uri")
		return nil
	}

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan error, 1)
	go func() {
		_, err := bridge.LoginWithOidc(ctx)
		done <- err
	}()
	redirectURI := <-opened
	cancel()
	if err := <-done; !errors.Is(err, context.Canceled) {
		t.Fatalf("cancelled LoginWithOidc = %v", err)
	}
	if response, err := http.Get(redirectURI); err == nil {
		_ = response.Body.Close()
		t.Fatal("loopback listener still open after cancellation")
	}

	previousTimeout := oidcSignInTimeout
	oidcSignInTimeout = 50 * time.Millisecond
	defer func() { oidcSignInTimeout = previousTimeout }()
	_, err := bridge.LoginWithOidc(context.Background())
	if errorCode(err) != CodeOidcTimeout {
		t.Fatalf("timed out LoginWithOidc = %v", err)
	}
	<-opened
	if fake.exchangeCount() != 0 {
		t.Fatal("an unfinished sign-in was exchanged")
	}
}

func TestLoginWithOidcRequiresPlatformSupport(t *testing.T) {
	server := httptest.NewTLSServer(http.NotFoundHandler())
	defer server.Close()
	bridge, _ := newTestBridge(t, server, newMemorySecretStore())
	if _, err := bridge.LoginWithOidc(context.Background()); errorCode(err) != CodeUnsupported {
		t.Fatalf("LoginWithOidc without callbacks = %v", err)
	}
	if bridge.ConsumeOidcInterruption() {
		t.Fatal("interruption reported without callbacks")
	}
}

func TestAppLinkCallbacksDeliverOnlyTheMatchingLink(t *testing.T) {
	callbacks := NewAppLinkOidcCallbacks()
	listener, _ := callbacks.Listen("state-0123456789abcdef")
	defer listener.Close()
	if listener.RedirectURI() != AppLinkOidcRedirectURI {
		t.Fatalf("redirect URI = %q", listener.RedirectURI())
	}
	for _, link := range []string{
		"https://oidc/callback?state=state-0123456789abcdef&code=x",
		"litradar://oidc/other?state=state-0123456789abcdef&code=x",
		"litradar://evil/callback?state=state-0123456789abcdef&code=x",
		"litradar://oidc/callback?state=other-state-0123456789&code=x",
		"litradar://oidc/callback?code=x",
	} {
		if callbacks.Deliver(link) {
			t.Fatalf("Deliver accepted %q", link)
		}
	}
	if !callbacks.Deliver("litradar://oidc/callback?code=good&state=state-0123456789abcdef") {
		t.Fatal("Deliver rejected the matching link")
	}
	query, err := listener.Wait(context.Background())
	if err != nil || query.Get("code") != "good" {
		t.Fatalf("Wait = %v, %v", query, err)
	}
	if callbacks.Deliver("litradar://oidc/callback?code=again&state=state-0123456789abcdef") {
		t.Fatal("a second callback was accepted")
	}
	if callbacks.ConsumeInterruption() {
		t.Fatal("a callback after a completed sign-in reported an interruption")
	}
}

func TestAppLinkCallbacksReportLostSignInsOnce(t *testing.T) {
	callbacks := NewAppLinkOidcCallbacks()
	if callbacks.Deliver("litradar://oidc/callback?code=x&state=lost-state-0123456789") {
		t.Fatal("a callback without a sign-in was accepted")
	}
	if !callbacks.ConsumeInterruption() || callbacks.ConsumeInterruption() {
		t.Fatal("a callback after a restart must report one interruption")
	}

	listener, _ := callbacks.Listen("state-0123456789abcdef")
	waitResult := make(chan error, 1)
	go func() {
		_, err := listener.Wait(context.Background())
		waitResult <- err
	}()
	callbacks.Restart()
	if err := <-waitResult; !errors.Is(err, errOidcSuperseded) {
		t.Fatalf("Wait after Restart = %v", err)
	}
	if callbacks.Deliver("litradar://oidc/callback?code=x&state=state-0123456789abcdef") || !callbacks.ConsumeInterruption() {
		t.Fatal("a callback for the previous activity's sign-in must report an interruption")
	}
}

func TestAppLinkCallbacksIgnoreCallbacksOfCancelledSignIns(t *testing.T) {
	callbacks := NewAppLinkOidcCallbacks()
	first, _ := callbacks.Listen("first-state-0123456789")
	second, _ := callbacks.Listen("second-state-0123456789")
	if _, err := first.Wait(context.Background()); !errors.Is(err, errOidcSuperseded) {
		t.Fatalf("superseded Wait = %v", err)
	}
	second.Close()
	if callbacks.Deliver("litradar://oidc/callback?code=x&state=second-state-0123456789") {
		t.Fatal("a cancelled sign-in accepted its callback")
	}
	if callbacks.ConsumeInterruption() {
		t.Fatal("a cancelled sign-in reported an interruption")
	}
}
