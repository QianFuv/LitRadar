package mcpcompat

import (
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestHostOriginGuardPrecedesMediaAndJson(t *testing.T) {
	policy := HostOriginPolicy{AllowedHosts: []string{"localhost", "127.0.0.1", "::1", "example.test:8443"}, AllowedOrigins: []string{"https://app.test", "https://exact.test:443", "null"}}
	server := mcp.NewServer(&mcp.Implementation{Name: "fixture", Version: "1"}, nil)
	handler := NewWithPolicy(server, func(writer http.ResponseWriter, request *http.Request) (Principal, bool) {
		if request.Header.Get("Authorization") != "Bearer fixture" {
			writer.WriteHeader(401)
			io.WriteString(writer, "unauthorized")
			return Principal{}, false
		}
		return Principal{UserId: 1, ExpiresAt: time.Now().Add(time.Hour)}, true
	}, policy)
	listener := httptest.NewServer(handler)
	defer listener.Close()
	defer handler.Close()
	for _, scenario := range []struct {
		name, host, origin string
		unauthorized       bool
		status             int
		body, contentType  string
	}{
		{"authentication", "evil.test", "broken", true, 401, "unauthorized", "text/plain; charset=utf-8"},
		{"host-before-origin", "evil.test", "broken", false, 403, "Forbidden: Host header is not allowed", ""},
		{"origin-before-accept", "localhost:9999", "http://other.test", false, 403, "Forbidden: Origin header is not allowed", ""},
		{"origin-invalid", "localhost", "broken", false, 400, "Bad Request: Invalid Origin header", "text/plain; charset=utf-8"},
		{"case-any-port", "LOCALHOST:9000", "HTTPS://APP.TEST:123/path?q=1", false, 406, "Not Acceptable: Client must accept both application/json and text/event-stream", ""},
		{"ipv6", "[::1]:8000", "NULL", false, 406, "Not Acceptable: Client must accept both application/json and text/event-stream", ""},
		{"no-trailing-dot-normalization", "localhost.", "", false, 403, "Forbidden: Host header is not allowed", ""},
		{"explicit-host-port", "example.test:8443", "", false, 406, "Not Acceptable: Client must accept both application/json and text/event-stream", ""},
		{"wrong-host-port", "example.test:8444", "", false, 403, "Forbidden: Host header is not allowed", ""},
		{"origin-port-absent", "localhost", "https://exact.test", false, 403, "Forbidden: Origin header is not allowed", ""},
		{"origin-port-exact", "localhost", "https://exact.test:443/path", false, 406, "Not Acceptable: Client must accept both application/json and text/event-stream", ""},
		{"positive-port", "example.test:+8443", "https://exact.test:+443", false, 406, "Not Acceptable: Client must accept both application/json and text/event-stream", ""},
		{"invalid-path", "localhost", "https://app.test/<", false, 400, "Bad Request: Invalid Origin header", "text/plain; charset=utf-8"},
		{"invalid-query", "localhost", "https://app.test/?q=\"", false, 400, "Bad Request: Invalid Origin header", "text/plain; charset=utf-8"},
		{"ignored-fragment", "localhost", "https://app.test/#x y", false, 406, "Not Acceptable: Client must accept both application/json and text/event-stream", ""},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			request, _ := http.NewRequest("POST", listener.URL, strings.NewReader("{"))
			request.Host = scenario.host
			if !scenario.unauthorized {
				request.Header.Set("Authorization", "Bearer fixture")
			}
			if scenario.origin != "" {
				request.Header.Set("Origin", scenario.origin)
			}
			response, err := listener.Client().Do(request)
			if err != nil {
				t.Fatal(err)
			}
			body, err := io.ReadAll(response.Body)
			response.Body.Close()
			if err != nil || response.StatusCode != scenario.status || string(body) != scenario.body || response.Header.Get("Content-Type") != scenario.contentType {
				t.Fatalf("response %d %q %q %v", response.StatusCode, body, response.Header.Get("Content-Type"), err)
			}
		})
	}
}

func TestLegacyAuthorityAndSchemeSyntax(t *testing.T) {
	for _, value := range []string{"1://app.test", "a~://app.test", "+://app.test"} {
		if _, ok := parseOrigin(value); !ok {
			t.Errorf("legacy scheme rejected: %s", value)
		}
	}
	policy := HostOriginPolicy{AllowedHosts: []string{"example.test:+80"}}
	request := httptest.NewRequest("POST", "http://example.test:81/", nil)
	if policy.validate(httptest.NewRecorder(), request) {
		t.Fatal("explicit positive port became wildcard")
	}
}

func TestEmptyOriginPolicyAndPresentEmptyHost(t *testing.T) {
	policy := HostOriginPolicy{}
	request := httptest.NewRequest("POST", "http://localhost/", strings.NewReader("{"))
	request.Header.Set("Origin", "not even a URI")
	if !policy.validate(httptest.NewRecorder(), request) {
		t.Fatal("empty origin policy parsed Origin")
	}
	request.Header.Set("Host", "")
	writer := httptest.NewRecorder()
	if policy.validate(writer, request) || writer.Code != 400 || writer.Body.String() != "Bad Request: Invalid Host header" {
		t.Fatalf("empty present Host %d %q", writer.Code, writer.Body.String())
	}
}
