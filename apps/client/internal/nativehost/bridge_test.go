package nativehost

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

type memorySecretStore struct {
	mutex  sync.Mutex
	values map[string]string
}

func newMemorySecretStore() *memorySecretStore {
	return &memorySecretStore{values: map[string]string{}}
}

func (store *memorySecretStore) Get(key string) (string, bool, error) {
	store.mutex.Lock()
	defer store.mutex.Unlock()
	value, ok := store.values[key]
	return value, ok, nil
}

func (store *memorySecretStore) Set(key, value string) error {
	store.mutex.Lock()
	defer store.mutex.Unlock()
	store.values[key] = value
	return nil
}

func (store *memorySecretStore) Delete(key string) error {
	store.mutex.Lock()
	defer store.mutex.Unlock()
	delete(store.values, key)
	return nil
}

// newTestBridge returns a bridge configured for server with the given secrets.
func newTestBridge(t *testing.T, server *httptest.Server, secrets SecretStore) (*Bridge, string) {
	t.Helper()
	directory := t.TempDir()
	client := server.Client()
	client.CheckRedirect = func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }
	bridge := NewBridge(Options{
		Secrets:         secrets,
		ConfigDirectory: func() (string, error) { return directory, nil },
		DeviceName:      "Windows test-host",
		Client:          client,
	})
	origin, err := NormalizeServerOrigin(server.URL)
	if err != nil {
		t.Fatalf("normalize test origin: %v", err)
	}
	if err := NewConfigStore(directory).Save(origin); err != nil {
		t.Fatalf("save test origin: %v", err)
	}
	return bridge, origin
}

func errorCode(err error) string {
	if bridgeError, ok := errors.AsType[*BridgeError](err); ok {
		return bridgeError.Code
	}
	return ""
}

func TestNormalizeServerOrigin(t *testing.T) {
	cases := []struct {
		name    string
		raw     string
		want    string
		wantErr string
	}{
		{name: "https host", raw: "https://litradar.example.com", want: "https://litradar.example.com"},
		{name: "case and trailing slash", raw: " HTTPS://LitRadar.Example.com/ ", want: "https://litradar.example.com"},
		{name: "default port", raw: "https://example.com:443", want: "https://example.com"},
		{name: "custom port", raw: "https://example.com:8443", want: "https://example.com:8443"},
		{name: "http ipv4 loopback", raw: "http://127.0.0.1:8001", want: "http://127.0.0.1:8001"},
		{name: "http localhost", raw: "http://localhost:8001", want: "http://localhost:8001"},
		{name: "http ipv6 loopback", raw: "http://[::1]:8001", want: "http://[::1]:8001"},
		{name: "http remote host", raw: "http://example.com", wantErr: CodeInsecureOrigin},
		{name: "http emulator host alias", raw: "http://10.0.2.2:8001", wantErr: CodeInsecureOrigin},
		{name: "path", raw: "https://example.com/litradar", wantErr: CodeInvalidOrigin},
		{name: "query", raw: "https://example.com/?next=1", wantErr: CodeInvalidOrigin},
		{name: "credentials", raw: "https://user:secret@example.com", wantErr: CodeInvalidOrigin},
		{name: "other scheme", raw: "ftp://example.com", wantErr: CodeInvalidOrigin},
		{name: "no scheme", raw: "example.com", wantErr: CodeInvalidOrigin},
		{name: "empty", raw: "  ", wantErr: CodeInvalidOrigin},
	}
	for _, testCase := range cases {
		t.Run(testCase.name, func(t *testing.T) {
			origin, err := NormalizeServerOrigin(testCase.raw)
			if testCase.wantErr != "" {
				if errorCode(err) != testCase.wantErr {
					t.Fatalf("NormalizeServerOrigin(%q) error = %v, want code %s", testCase.raw, err, testCase.wantErr)
				}
				return
			}
			if err != nil || origin != testCase.want {
				t.Fatalf("NormalizeServerOrigin(%q) = %q, %v; want %q", testCase.raw, origin, err, testCase.want)
			}
		})
	}
}

func TestResolveServerURLRestrictsOriginAndPath(t *testing.T) {
	const origin = "https://litradar.example.com"
	cases := []struct {
		rawURL    string
		isAllowed bool
	}{
		{rawURL: "https://litradar.example.com/api/auth/me", isAllowed: true},
		{rawURL: "https://LITRADAR.example.com:443/api/search?q=a", isAllowed: true},
		{rawURL: "https://litradar.example.com/health/ready", isAllowed: true},
		{rawURL: "https://litradar.example.com/health/live", isAllowed: false},
		{rawURL: "https://litradar.example.com/mcp", isAllowed: false},
		{rawURL: "https://litradar.example.com/api", isAllowed: false},
		{rawURL: "https://litradar.example.com/api/../admin", isAllowed: false},
		{rawURL: "https://litradar.example.com/api/%2e%2e/admin", isAllowed: false},
		{rawURL: "https://litradar.example.com/api/a%2Fb", isAllowed: false},
		{rawURL: "https://other.example.com/api/auth/me", isAllowed: false},
		{rawURL: "https://litradar.example.com:8443/api/auth/me", isAllowed: false},
		{rawURL: "http://litradar.example.com/api/auth/me", isAllowed: false},
		{rawURL: "/api/auth/me", isAllowed: false},
	}
	for _, testCase := range cases {
		_, err := resolveServerURL(origin, testCase.rawURL)
		if testCase.isAllowed && err != nil {
			t.Errorf("resolveServerURL(%q) rejected: %v", testCase.rawURL, err)
		}
		if !testCase.isAllowed && errorCode(err) != CodeForbiddenTarget {
			t.Errorf("resolveServerURL(%q) error = %v, want %s", testCase.rawURL, err, CodeForbiddenTarget)
		}
	}
}

func TestSendInjectsBearerAndFiltersHeaders(t *testing.T) {
	var received http.Header
	var receivedBody string
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		received = request.Header.Clone()
		body, _ := io.ReadAll(request.Body)
		receivedBody = string(body)
		writer.Header().Set("Content-Type", "application/json")
		writer.Header().Set("X-Request-Id", "request-1")
		writer.Header().Set("Set-Cookie", "session=leak")
		writer.WriteHeader(http.StatusCreated)
		_, _ = writer.Write([]byte(`{"ok":true}`))
	}))
	defer server.Close()
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)
	_ = secrets.Set(tokenKey(origin), "token-for-origin")

	response, err := bridge.Send(context.Background(), Request{
		Method: "post",
		URL:    origin + "/api/favorites/folders",
		Headers: map[string]string{
			"content-type":  "application/json",
			"Authorization": "Bearer forged",
			"Cookie":        "session=forged",
			"X-Custom":      "dropped",
		},
		Body: `{"name":"reading"}`,
	})
	if err != nil {
		t.Fatalf("Send: %v", err)
	}
	if got := received.Get("Authorization"); got != "Bearer token-for-origin" {
		t.Errorf("Authorization = %q, want stored bearer token", got)
	}
	if received.Get("Cookie") != "" || received.Get("X-Custom") != "" {
		t.Errorf("unexpected forwarded headers: %v", received)
	}
	if received.Get("Content-Type") != "application/json" || receivedBody != `{"name":"reading"}` {
		t.Errorf("content type or body not forwarded: %q %q", received.Get("Content-Type"), receivedBody)
	}
	if response.Status != http.StatusCreated || string(response.Body) != `{"ok":true}` {
		t.Errorf("response = %d %q", response.Status, response.Body)
	}
	if response.Headers["X-Request-Id"] != "request-1" || response.Headers["Content-Type"] != "application/json" {
		t.Errorf("exposed headers missing: %v", response.Headers)
	}
	if _, ok := response.Headers["Set-Cookie"]; ok {
		t.Errorf("Set-Cookie must not be exposed: %v", response.Headers)
	}
}

func TestSendUsesOnlyTheTokenOfTheConfiguredOrigin(t *testing.T) {
	var authorization string
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		authorization = request.Header.Get("Authorization")
		writer.WriteHeader(http.StatusUnauthorized)
	}))
	defer server.Close()
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)
	_ = secrets.Set(tokenKey("https://other.example.com"), "other-token")

	response, err := bridge.Send(context.Background(), Request{Method: "GET", URL: origin + "/api/auth/me"})
	if err != nil {
		t.Fatalf("Send: %v", err)
	}
	if authorization != "" || response.Status != http.StatusUnauthorized {
		t.Fatalf("authorization = %q, status = %d; want no bearer and 401", authorization, response.Status)
	}
}

func TestSendRejectsRequestsOutsideTheServerAPI(t *testing.T) {
	var hits int
	server := httptest.NewTLSServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { hits++ }))
	defer server.Close()
	bridge, origin := newTestBridge(t, server, newMemorySecretStore())

	for _, target := range []string{"https://evil.example.com/api/auth/me", origin + "/mcp", origin + "/api/../health/live"} {
		_, err := bridge.Send(context.Background(), Request{Method: "GET", URL: target})
		if errorCode(err) != CodeForbiddenTarget {
			t.Errorf("Send(%q) error = %v, want %s", target, err, CodeForbiddenTarget)
		}
	}
	_, err := bridge.Send(context.Background(), Request{Method: "CONNECT", URL: origin + "/api/auth/me"})
	if errorCode(err) != CodeInvalidRequest {
		t.Errorf("CONNECT error = %v, want %s", err, CodeInvalidRequest)
	}
	if hits != 0 {
		t.Fatalf("server received %d rejected requests", hits)
	}
}

func TestSendDoesNotFollowRedirects(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path == "/api/start" {
			http.Redirect(writer, request, "https://evil.example.com/api/steal", http.StatusFound)
			return
		}
		t.Errorf("redirect target was requested: %s", request.URL)
	}))
	defer server.Close()
	bridge, origin := newTestBridge(t, server, newMemorySecretStore())

	response, err := bridge.Send(context.Background(), Request{Method: "GET", URL: origin + "/api/start"})
	if err != nil || response.Status != http.StatusFound {
		t.Fatalf("Send = %d, %v; want unfollowed 302", response.Status, err)
	}
}

func TestSendPropagatesCancellation(t *testing.T) {
	started := make(chan struct{})
	serverCancelled := make(chan struct{})
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		close(started)
		<-request.Context().Done()
		close(serverCancelled)
	}))
	defer server.Close()
	bridge, origin := newTestBridge(t, server, newMemorySecretStore())

	ctx, cancel := context.WithCancel(context.Background())
	result := make(chan error, 1)
	go func() {
		_, err := bridge.Send(ctx, Request{Method: "GET", URL: origin + "/api/slow"})
		result <- err
	}()
	<-started
	cancel()
	select {
	case err := <-result:
		if !errors.Is(err, context.Canceled) {
			t.Fatalf("Send error = %v, want context.Canceled", err)
		}
	case <-time.After(5 * time.Second):
		t.Fatal("Send did not return after cancellation")
	}
	select {
	case <-serverCancelled:
	case <-time.After(5 * time.Second):
		t.Fatal("server request was not cancelled")
	}
}

func TestSendRequiresConfiguredServer(t *testing.T) {
	bridge := NewBridge(Options{
		Secrets:         newMemorySecretStore(),
		ConfigDirectory: func() (string, error) { return t.TempDir(), nil },
	})
	_, err := bridge.Send(context.Background(), Request{Method: "GET", URL: "https://example.com/api/auth/me"})
	if errorCode(err) != CodeServerNotConfigured {
		t.Fatalf("Send error = %v, want %s", err, CodeServerNotConfigured)
	}
}

func TestLoginStoresTokenAndHidesItFromTheFrontend(t *testing.T) {
	var loginRequest map[string]string
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/api/auth/native/login" || request.Header.Get("Authorization") != "" {
			t.Errorf("unexpected login request %s %q", request.URL.Path, request.Header.Get("Authorization"))
		}
		_ = json.NewDecoder(request.Body).Decode(&loginRequest)
		writer.Header().Set("Content-Type", "application/json")
		_, _ = writer.Write([]byte(`{"token":"issued-token","expires_at":1900000000,"user":{"id":7,"username":"member"}}`))
	}))
	defer server.Close()
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)

	response, err := bridge.Login(context.Background(), "member", "secret-password")
	if err != nil {
		t.Fatalf("Login: %v", err)
	}
	if loginRequest["username"] != "member" || loginRequest["password"] != "secret-password" ||
		loginRequest["device_name"] != "Windows test-host" {
		t.Errorf("login request = %v", loginRequest)
	}
	if strings.Contains(string(response.Body), "issued-token") {
		t.Fatalf("login response leaks the token: %s", response.Body)
	}
	var body map[string]json.RawMessage
	if err := json.Unmarshal(response.Body, &body); err != nil || string(body["expires_at"]) != "1900000000" ||
		!strings.Contains(string(body["user"]), `"member"`) {
		t.Fatalf("login response body = %s", response.Body)
	}
	if token, _, _ := secrets.Get(tokenKey(origin)); token != "issued-token" {
		t.Fatalf("stored token = %q", token)
	}
}

func TestLoginFailureReturnsServerErrorWithoutStoringAToken(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "application/json")
		writer.Header().Set("Retry-After", "30")
		writer.WriteHeader(http.StatusTooManyRequests)
		_, _ = writer.Write([]byte(`{"detail":"too many attempts"}`))
	}))
	defer server.Close()
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)

	response, err := bridge.Login(context.Background(), "member", "wrong")
	if err != nil || response.Status != http.StatusTooManyRequests || response.Headers["Retry-After"] != "30" {
		t.Fatalf("Login = %+v, %v", response, err)
	}
	if _, hasToken, _ := secrets.Get(tokenKey(origin)); hasToken {
		t.Fatal("failed login stored a token")
	}
}

func TestLogoutRevokesAndDeletesTheToken(t *testing.T) {
	var authorization string
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		authorization = request.Header.Get("Authorization")
		_, _ = writer.Write([]byte(`{"ok":true,"user_id":7}`))
	}))
	defer server.Close()
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)
	_ = secrets.Set(tokenKey(origin), "session-token")

	response, err := bridge.Logout(context.Background())
	if err != nil || response.Status != http.StatusOK || authorization != "Bearer session-token" {
		t.Fatalf("Logout = %d, %v, authorization %q", response.Status, err, authorization)
	}
	if _, hasToken, _ := secrets.Get(tokenKey(origin)); hasToken {
		t.Fatal("token survived logout")
	}
}

func TestLogoutDeletesTheTokenWhenTheServerIsUnreachable(t *testing.T) {
	server := httptest.NewTLSServer(http.HandlerFunc(func(http.ResponseWriter, *http.Request) {}))
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)
	_ = secrets.Set(tokenKey(origin), "session-token")
	server.Close()

	_, err := bridge.Logout(context.Background())
	if errorCode(err) != CodeServerUnreachable {
		t.Fatalf("Logout error = %v, want %s", err, CodeServerUnreachable)
	}
	if _, hasToken, _ := secrets.Get(tokenKey(origin)); hasToken {
		t.Fatal("token survived an unconfirmed logout")
	}
}

func TestConfigureServerProbesReadinessAndPersistsTheOrigin(t *testing.T) {
	readiness := http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/health/ready" {
			http.NotFound(writer, request)
			return
		}
		_, _ = writer.Write([]byte(`{"status":"ok"}`))
	})
	server := httptest.NewServer(readiness)
	defer server.Close()
	directory := t.TempDir()
	options := Options{
		Secrets:         newMemorySecretStore(),
		ConfigDirectory: func() (string, error) { return directory, nil },
	}

	origin, err := NewBridge(options).ConfigureServer(context.Background(), server.URL+"/")
	if err != nil || origin != server.URL {
		t.Fatalf("ConfigureServer = %q, %v; want loopback http origin %q", origin, err, server.URL)
	}
	restored, err := NewBridge(options).ServerOrigin()
	if err != nil || restored != server.URL {
		t.Fatalf("restored origin = %q, %v", restored, err)
	}
}

func TestConfigureServerRejectsUnreadyAndForeignServers(t *testing.T) {
	cases := []struct {
		name     string
		status   int
		body     string
		wantCode string
	}{
		{name: "unready", status: http.StatusServiceUnavailable, body: `{"status":"unhealthy"}`, wantCode: CodeServerNotReady},
		{name: "foreign", status: http.StatusOK, body: `<html></html>`, wantCode: CodeNotLitRadarServer},
	}
	for _, testCase := range cases {
		t.Run(testCase.name, func(t *testing.T) {
			server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) {
				writer.WriteHeader(testCase.status)
				_, _ = writer.Write([]byte(testCase.body))
			}))
			defer server.Close()
			bridge, _ := newTestBridge(t, server, newMemorySecretStore())
			if _, err := bridge.ConfigureServer(context.Background(), server.URL); errorCode(err) != testCase.wantCode {
				t.Fatalf("ConfigureServer error = %v, want %s", err, testCase.wantCode)
			}
		})
	}
}

func TestForgetServerRevokesDeletesAndClears(t *testing.T) {
	var revoked string
	server := httptest.NewTLSServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		if request.URL.Path == "/api/auth/logout" {
			revoked = request.Header.Get("Authorization")
		}
		_, _ = writer.Write([]byte(`{"ok":true}`))
	}))
	defer server.Close()
	secrets := newMemorySecretStore()
	bridge, origin := newTestBridge(t, server, secrets)
	_ = secrets.Set(tokenKey(origin), "session-token")
	_ = secrets.Set(tokenKey("https://other.example.com"), "other-token")

	if err := bridge.ForgetServer(context.Background()); err != nil {
		t.Fatalf("ForgetServer: %v", err)
	}
	if revoked != "Bearer session-token" {
		t.Errorf("revocation authorization = %q", revoked)
	}
	if _, hasToken, _ := secrets.Get(tokenKey(origin)); hasToken {
		t.Error("token of the forgotten server survived")
	}
	if _, hasToken, _ := secrets.Get(tokenKey("https://other.example.com")); !hasToken {
		t.Error("token of another server was deleted")
	}
	if current, err := bridge.ServerOrigin(); err != nil || current != "" {
		t.Errorf("origin after forget = %q, %v", current, err)
	}
	if _, err := bridge.Send(context.Background(), Request{Method: "GET", URL: origin + "/api/auth/me"}); errorCode(err) != CodeServerNotConfigured {
		t.Errorf("Send after forget error = %v", err)
	}
}

func TestSaveDownloadAndOpenExternalValidateInput(t *testing.T) {
	var savedName string
	var openedURL string
	bridge := NewBridge(Options{
		Secrets:         newMemorySecretStore(),
		ConfigDirectory: func() (string, error) { return t.TempDir(), nil },
		SaveFile: func(_ context.Context, filename string, _ []byte) (bool, error) {
			savedName = filename
			return true, nil
		},
		OpenURL: func(rawURL string) error {
			openedURL = rawURL
			return nil
		},
	})
	if saved, err := bridge.SaveDownload(context.Background(), `..\..\favorites.bib`, []byte("@article{}")); err != nil || !saved {
		t.Fatalf("SaveDownload = %v, %v", saved, err)
	}
	if savedName != "favorites.bib" {
		t.Errorf("saved name = %q, want directories stripped", savedName)
	}
	if err := bridge.OpenExternal("javascript:alert(1)"); errorCode(err) != CodeInvalidRequest {
		t.Errorf("OpenExternal(javascript) error = %v", err)
	}
	if err := bridge.OpenExternal("https://doi.org/10.1000/1"); err != nil || openedURL != "https://doi.org/10.1000/1" {
		t.Errorf("OpenExternal(https) = %v, opened %q", err, openedURL)
	}
	unsupported := NewBridge(Options{Secrets: newMemorySecretStore(), ConfigDirectory: func() (string, error) { return "", nil }})
	if _, err := unsupported.SaveDownload(context.Background(), "a.txt", nil); errorCode(err) != CodeUnsupported {
		t.Errorf("SaveDownload without dialog error = %v", err)
	}
}

func TestSanitizeDeviceName(t *testing.T) {
	long := strings.Repeat("设", 120)
	cases := map[string]string{
		"Windows DESKTOP-1":      "Windows DESKTOP-1",
		"  Android \x00 Pixel\n": "Android Pixel",
		"":                       fallbackDeviceName,
		long:                     strings.Repeat("设", maxDeviceNameRunes),
	}
	for input, want := range cases {
		if got := sanitizeDeviceName(input); got != want {
			t.Errorf("sanitizeDeviceName(%q) = %q, want %q", input, got, want)
		}
	}
}
