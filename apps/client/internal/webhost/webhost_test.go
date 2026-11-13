package webhost

import (
	"net/http"
	"net/http/httptest"
	"testing"
	"testing/fstest"
)

const testIndex = "<!doctype html><title>LitRadar</title>"

func newTestHost(t *testing.T, config Config) http.Handler {
	t.Helper()
	dist := fstest.MapFS{
		"index.html":               {Data: []byte(testIndex)},
		"assets/index-abc123.js":   {Data: []byte("console.log(1)")},
		"assets/font-def456.woff2": {Data: []byte("wOF2")},
		"theme-init.js":            {Data: []byte("(function(){})()")},
		"litradar-icon.svg":        {Data: []byte("<svg/>")},
	}
	handler, err := NewHandler(dist)
	if err != nil {
		t.Fatalf("NewHandler() error = %v", err)
	}
	return SecurityMiddleware(config)(handler)
}

func serve(handler http.Handler, method, target string) *httptest.ResponseRecorder {
	recorder := httptest.NewRecorder()
	handler.ServeHTTP(recorder, httptest.NewRequest(method, target, nil))
	return recorder
}

func TestHandlerServesIndexForClientRoutes(t *testing.T) {
	host := newTestHost(t, Config{})
	for _, target := range []string{"/", "/index.html", "/login", "/login?next=%2F", "/unknown/deep/route"} {
		response := serve(host, http.MethodGet, target)
		if response.Code != http.StatusOK {
			t.Fatalf("GET %s status = %d, want 200", target, response.Code)
		}
		if response.Body.String() != testIndex {
			t.Fatalf("GET %s body = %q, want index.html", target, response.Body.String())
		}
		if got := response.Header().Get("Cache-Control"); got != cacheRevalidate {
			t.Fatalf("GET %s Cache-Control = %q, want %q", target, got, cacheRevalidate)
		}
		if got := response.Header().Get("Content-Type"); got != "text/html; charset=utf-8" {
			t.Fatalf("GET %s Content-Type = %q", target, got)
		}
	}
}

func TestHandlerCachesAssetsByName(t *testing.T) {
	host := newTestHost(t, Config{})
	cases := []struct {
		target, cacheControl, contentType string
	}{
		{"/assets/index-abc123.js", cacheImmutable, "text/javascript; charset=utf-8"},
		{"/assets/font-def456.woff2", cacheImmutable, "font/woff2"},
		{"/theme-init.js", cacheRevalidate, "text/javascript; charset=utf-8"},
		{"/litradar-icon.svg", cacheStatic, "image/svg+xml"},
	}
	for _, testCase := range cases {
		response := serve(host, http.MethodGet, testCase.target)
		if response.Code != http.StatusOK {
			t.Fatalf("GET %s status = %d, want 200", testCase.target, response.Code)
		}
		if got := response.Header().Get("Cache-Control"); got != testCase.cacheControl {
			t.Fatalf("GET %s Cache-Control = %q, want %q", testCase.target, got, testCase.cacheControl)
		}
		if got := response.Header().Get("Content-Type"); got != testCase.contentType {
			t.Fatalf("GET %s Content-Type = %q, want %q", testCase.target, got, testCase.contentType)
		}
	}
}

func TestHandlerKeepsMissingFilesNotFound(t *testing.T) {
	host := newTestHost(t, Config{})
	for _, target := range []string{"/assets/missing.js", "/assets/chunk", "/favicon.ico", "/../index.html.bak"} {
		if response := serve(host, http.MethodGet, target); response.Code != http.StatusNotFound {
			t.Fatalf("GET %s status = %d, want 404", target, response.Code)
		}
	}
}

func TestHandlerRejectsMutatingMethods(t *testing.T) {
	host := newTestHost(t, Config{})
	response := serve(host, http.MethodPost, "/login")
	if response.Code != http.StatusMethodNotAllowed {
		t.Fatalf("POST /login status = %d, want 405", response.Code)
	}
	if got := response.Header().Get("Allow"); got != "GET, HEAD" {
		t.Fatalf("Allow = %q", got)
	}
}

func TestMiddlewareHidesWailsRuntime(t *testing.T) {
	host := newTestHost(t, Config{})
	for _, target := range []string{"/wails", "/wails/runtime", "/wails/runtime.js", "/wails/transport.js", "/wails/stream/poll"} {
		for _, method := range []string{http.MethodGet, http.MethodPost} {
			response := serve(host, method, target)
			if response.Code != http.StatusNotFound {
				t.Fatalf("%s %s status = %d, want 404", method, target, response.Code)
			}
			if response.Header().Get("Content-Security-Policy") == "" {
				t.Fatalf("%s %s is missing security headers", method, target)
			}
		}
	}
}

func TestMiddlewareSetsSecurityHeaders(t *testing.T) {
	response := serve(newTestHost(t, Config{}), http.MethodGet, "/login")
	want := map[string]string{
		"Content-Security-Policy": ContentSecurityPolicy,
		"X-Content-Type-Options":  "nosniff",
		"Referrer-Policy":         "same-origin",
		"Permissions-Policy":      permissionsPolicy,
		"X-Frame-Options":         "DENY",
	}
	for name, value := range want {
		if got := response.Header().Get(name); got != value {
			t.Fatalf("%s = %q, want %q", name, got, value)
		}
	}
	if got := response.Header().Get("Strict-Transport-Security"); got != "" {
		t.Fatalf("Strict-Transport-Security = %q, want absent by default", got)
	}

	hsts := serve(newTestHost(t, Config{HstsMaxAge: 63072000}), http.MethodGet, "/")
	if got := hsts.Header().Get("Strict-Transport-Security"); got != "max-age=63072000" {
		t.Fatalf("Strict-Transport-Security = %q, want max-age=63072000", got)
	}
}

func TestNewHandlerRequiresIndex(t *testing.T) {
	if _, err := NewHandler(fstest.MapFS{}); err == nil {
		t.Fatal("NewHandler() error = nil, want missing index error")
	}
}

func TestLoadConfig(t *testing.T) {
	environment := map[string]string{
		EnvServerHost:     " 0.0.0.0 ",
		EnvServerPort:     "9000",
		EnvOriginPatterns: "https://litradar.example, ,*.litradar.example",
		EnvHstsMaxAge:     "31536000",
	}
	config, err := LoadConfig(func(name string) (string, bool) {
		value, ok := environment[name]
		return value, ok
	})
	if err != nil {
		t.Fatalf("LoadConfig() error = %v", err)
	}
	if config.Host != "0.0.0.0" || config.Port != 9000 || config.HstsMaxAge != 31536000 {
		t.Fatalf("LoadConfig() = %+v", config)
	}
	if len(config.OriginPatterns) != 2 || config.OriginPatterns[0] != "https://litradar.example" || config.OriginPatterns[1] != "*.litradar.example" {
		t.Fatalf("OriginPatterns = %q", config.OriginPatterns)
	}

	defaults, err := LoadConfig(func(string) (string, bool) { return "", false })
	if err != nil || defaults.Host != DefaultHost || defaults.Port != DefaultPort || defaults.HstsMaxAge != 0 {
		t.Fatalf("LoadConfig() defaults = %+v, %v", defaults, err)
	}

	for _, invalid := range []map[string]string{
		{EnvServerPort: "0"},
		{EnvServerPort: "http"},
		{EnvHstsMaxAge: "-1"},
	} {
		if _, err := LoadConfig(func(name string) (string, bool) {
			value, ok := invalid[name]
			return value, ok
		}); err == nil {
			t.Fatalf("LoadConfig(%v) error = nil, want rejection", invalid)
		}
	}
}
