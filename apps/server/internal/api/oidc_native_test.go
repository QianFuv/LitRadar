package api

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/testkit/oidctest"
)

type nativeTestProvider struct {
	key                      *oidctest.Key
	mutex                    sync.Mutex
	issuer, nonce, challenge string
}

func (provider *nativeTestProvider) serve(writer http.ResponseWriter, request *http.Request) {
	provider.mutex.Lock()
	defer provider.mutex.Unlock()
	writer.Header().Set("Content-Type", "application/json")
	var body any
	switch request.URL.Path {
	case "/.well-known/openid-configuration":
		body = map[string]string{"issuer": provider.issuer, "authorization_endpoint": provider.issuer + "/authorize", "token_endpoint": provider.issuer + "/token", "userinfo_endpoint": provider.issuer + "/userinfo", "jwks_uri": provider.issuer + "/jwks"}
	case "/jwks":
		body = provider.key.JWKS()
	case "/token":
		if err := request.ParseForm(); err != nil {
			writer.WriteHeader(400)
			return
		}
		client, secret, hasAuth := request.BasicAuth()
		digest := sha256.Sum256([]byte(request.Form.Get("code_verifier")))
		if !hasAuth || client != "client" || secret != "secret" || base64.RawURLEncoding.EncodeToString(digest[:]) != provider.challenge || request.Form.Get("redirect_uri") != "https://paper.example/api/auth/oidc/callback" {
			writer.WriteHeader(400)
			return
		}
		claims, err := provider.key.Sign(map[string]any{"iss": provider.issuer, "sub": "native-subject", "aud": "client", "nonce": provider.nonce, "iat": time.Now().Unix(), "exp": time.Now().Add(time.Hour).Unix()})
		if err != nil {
			writer.WriteHeader(500)
			return
		}
		body = map[string]string{"access_token": "access", "token_type": "Bearer", "id_token": claims}
	case "/userinfo":
		if request.Header.Get("Authorization") != "Bearer access" {
			writer.WriteHeader(401)
			return
		}
		body = map[string]any{"sub": "native-subject", "email": "native@example.com", "email_verified": true}
	default:
		writer.WriteHeader(404)
		return
	}
	if err := json.NewEncoder(writer).Encode(body); err != nil {
		return
	}
}

func nativeAPIFixture(t *testing.T) (*authHandlers, *http.ServeMux, *nativeTestProvider) {
	t.Helper()
	handlers, router, _, codec := apiOidcFixture(t)
	provider := &nativeTestProvider{key: oidctest.NewKey(t)}
	server := httptest.NewServer(http.HandlerFunc(provider.serve))
	t.Cleanup(server.Close)
	provider.issuer = server.URL
	repository := settings.New(handlers.repository, codec, true)
	values := map[string]*string{}
	for field, value := range map[string]string{"oidc_issuer_url": server.URL, "oidc_client_id": "client", "oidc_client_secret": "secret", "public_base_url": "https://paper.example"} {
		values[field] = &value
	}
	if _, err := repository.Update(t.Context(), nil, values, nil, nil); err != nil {
		t.Fatal(err)
	}
	handlers.oidc = auth.NewOidc(handlers.service, repository, codec, true)
	return handlers, router, provider
}

func nativeLoginQuery(redirect string) (string, string) {
	verifier := strings.Repeat("v", 43)
	digest := sha256.Sum256([]byte(verifier))
	return "/api/auth/oidc/native/login?" + url.Values{"redirect_uri": {redirect}, "code_challenge": {base64.RawURLEncoding.EncodeToString(digest[:])}, "state": {"client_state-0123456789"}}.Encode(), verifier
}

func completeAPIOidc(t *testing.T, router http.Handler, provider *nativeTestProvider, startPath string) *httptest.ResponseRecorder {
	t.Helper()
	response := authRequest(router, "GET", startPath, "", "")
	if response.Code != 302 || len(response.Result().Cookies()) != 1 {
		t.Fatal(response.Code, response.Body.String())
	}
	location, err := url.Parse(response.Header().Get("Location"))
	if err != nil {
		t.Fatal(err)
	}
	provider.mutex.Lock()
	provider.nonce, provider.challenge = location.Query().Get("nonce"), location.Query().Get("code_challenge")
	provider.mutex.Unlock()
	request := httptest.NewRequest("GET", "/api/auth/oidc/callback?"+url.Values{"state": {location.Query().Get("state")}, "code": {"idp-code"}}.Encode(), nil)
	request.AddCookie(response.Result().Cookies()[0])
	completed := httptest.NewRecorder()
	router.ServeHTTP(completed, request)
	observeResponse(request, completed)
	if completed.Code != 302 {
		t.Fatal(completed.Code, completed.Body.String())
	}
	return completed
}

// TestNativeOidcHttpFlow covers the browser redirect, callback, exchange and unchanged web cookie.
func TestNativeOidcHttpFlow(t *testing.T) {
	for _, redirect := range []string{"http://127.0.0.1:41923/oidc/callback", "litradar://oidc/callback"} {
		t.Run(redirect, func(t *testing.T) {
			_, router, provider := nativeAPIFixture(t)
			web := completeAPIOidc(t, router, provider, "/api/auth/oidc/login?next=/favorites")
			if web.Header().Get("Location") != "/favorites" {
				t.Fatal("web next changed", web.Header())
			}
			var sessionCookie *http.Cookie
			for _, cookie := range web.Result().Cookies() {
				if cookie.Name == "litradar_session" {
					sessionCookie = cookie
				}
			}
			if sessionCookie == nil {
				t.Fatal("web OIDC did not issue session cookie")
			}
			start, verifier := nativeLoginQuery(redirect)
			callback := completeAPIOidc(t, router, provider, start)
			if callback.Header().Get("Set-Cookie") != "" {
				t.Fatal("native callback issued a cookie", callback.Header())
			}
			location, err := url.Parse(callback.Header().Get("Location"))
			if err != nil || location.Query().Get("state") != "client_state-0123456789" || location.Query().Get("error") != "" {
				t.Fatal(callback.Header(), err)
			}
			code, err := base64.RawURLEncoding.DecodeString(location.Query().Get("code"))
			if err != nil || len(code) != 32 {
				t.Fatal("invalid authorization code", err)
			}
			body, _ := json.Marshal(map[string]string{"code": location.Query().Get("code"), "code_verifier": verifier, "redirect_uri": redirect, "device_name": "Android"})
			exchange := authRequest(router, "POST", "/api/auth/oidc/native/exchange", string(body), "")
			var credential struct {
				Token     string       `json:"token"`
				ExpiresAt float64      `json:"expires_at"`
				User      userResponse `json:"user"`
			}
			if exchange.Code != 200 || json.Unmarshal(exchange.Body.Bytes(), &credential) != nil || credential.Token == "" || credential.User.AuthKind != "oidc" || exchange.Header().Get("Set-Cookie") != "" || exchange.Header().Get("Cache-Control") != "no-store" {
				t.Fatal(exchange.Code, exchange.Body.String())
			}
			if authRequest(router, "GET", "/api/auth/me", "", credential.Token).Code != 200 {
				t.Fatal("device token unusable")
			}
			reused := authRequest(router, "POST", "/api/auth/oidc/native/exchange", string(body), "")
			requireProblem(t, reused, 400, "invalid_grant")
			request := httptest.NewRequest("GET", "/api/auth/me", nil)
			request.AddCookie(sessionCookie)
			recorder := httptest.NewRecorder()
			router.ServeHTTP(recorder, request)
			if recorder.Code != 200 {
				t.Fatal("native flow invalidated web session", recorder.Code)
			}
		})
	}
}

// TestNativeOidcHttpAdmission checks disabled OIDC, strict query admission, generic grants and rate limits.
func TestNativeOidcHttpAdmission(t *testing.T) {
	handlers, router, _, codec := apiOidcFixture(t)
	start, _ := nativeLoginQuery("litradar://oidc/callback")
	if response := authRequest(router, "GET", start, "", ""); response.Code != 404 {
		t.Fatal(response.Code, response.Body.String())
	}
	for _, path := range []string{"/api/auth/oidc/native/login", start + "&state=duplicate", strings.Replace(start, "litradar%3A%2F%2Foidc%2Fcallback", "https%3A%2F%2Fevil.example", 1)} {
		if response := authRequest(router, "GET", path, "", ""); response.Code != map[bool]int{true: 400, false: 422}[strings.Contains(path, "evil.example")] || response.Header().Get("Location") != "" {
			t.Fatal(response.Code, response.Header(), response.Body.String())
		}
	}
	parameters := &auth.NativeOIDCParameters{RedirectURI: "litradar://oidc/callback", State: "client_state-0123456789", CodeChallenge: base64.RawURLEncoding.EncodeToString(make([]byte, 32))}
	for _, scenario := range []struct {
		query, expected string
		hasExpired      bool
	}{
		{"state=expected&error=access_denied", "denied", false},
		{"state=wrong&code=idp-code", "state", false},
		{"state=expected&error=server_error", "provider", false},
		{"state=expected&code=idp-code", "state", true},
	} {
		expires := time.Now().Add(time.Minute).Unix()
		if scenario.hasExpired {
			expires = time.Now().Add(-time.Second).Unix()
		}
		request := httptest.NewRequest("GET", "/api/auth/oidc/callback?"+scenario.query, nil)
		request.AddCookie(signedOidcCookie(t, codec, auth.OidcState{State: "expected", Nonce: "nonce", Verifier: "verifier", ExpiresAt: expires, Native: parameters}))
		response := httptest.NewRecorder()
		router.ServeHTTP(response, request)
		if response.Code != 302 || response.Header().Get("Location") != parameters.Redirect("error", scenario.expected) || response.Header().Get("Set-Cookie") != "" {
			t.Fatal(response.Code, response.Header())
		}
	}
	body := `{"code":"missing","code_verifier":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","redirect_uri":"litradar://oidc/callback","device_name":"Android"}`
	response := authRequest(router, "POST", "/api/auth/oidc/native/exchange", body, "")
	requireProblem(t, response, 400, "invalid_grant")
	policy := rateLimitTestPolicy()
	policy.LoginIP.Capacity = 1
	handlers.limiter = newAuthRateLimiter(policy)
	if response = authRequest(router, "POST", "/api/auth/oidc/native/exchange", body, ""); response.Code != 400 {
		t.Fatal(response.Code)
	}
	if response = authRequest(router, "POST", "/api/auth/oidc/native/exchange", body, ""); response.Code != 429 || response.Header().Get("Retry-After") == "" {
		t.Fatal(response.Code, response.Header())
	}
}

// TestNativeOidcStartProviderFailure returns control to a validated client when discovery fails.
func TestNativeOidcStartProviderFailure(t *testing.T) {
	handlers, router, _, codec := apiOidcFixture(t)
	server := httptest.NewServer(http.NotFoundHandler())
	t.Cleanup(server.Close)
	repository := settings.New(handlers.repository, codec, true)
	values := map[string]*string{}
	for field, value := range map[string]string{"oidc_issuer_url": server.URL, "oidc_client_id": "client", "oidc_client_secret": "secret", "public_base_url": "https://paper.example"} {
		values[field] = &value
	}
	if _, err := repository.Update(t.Context(), nil, values, nil, nil); err != nil {
		t.Fatal(err)
	}
	handlers.oidc = auth.NewOidc(handlers.service, repository, codec, true)
	path, _ := nativeLoginQuery("litradar://oidc/callback")
	response := authRequest(router, "GET", path, "", "")
	if response.Code != 302 || response.Header().Get("Location") != "litradar://oidc/callback?error=provider&state=client_state-0123456789" {
		t.Fatal(response.Code, response.Header())
	}
}
