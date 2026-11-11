package api

import (
	"bufio"
	"bytes"
	"context"

	"database/sql"

	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"

	"strings"
	"sync"
	"testing"
	"time"

	domainauth "github.com/QianFuv/LitRadar/apps/server/internal/domain/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/danielgtaylor/huma/v2"
	"github.com/danielgtaylor/huma/v2/adapters/humago"
)

const fixtureCsp = "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"

// fixtureMux installs synthetic handlers using the production router's matching semantics.
func fixtureMux(routes []route) *http.ServeMux {
	mux := http.NewServeMux()
	contract := openapi.New(mux)
	for _, candidate := range routes {
		if err := candidate.register(contract); err != nil {
			panic(err)
		}
	}
	return mux
}

var fixtureAPIs sync.Map

func registerFixtureRoutes(t *testing.T, mux *http.ServeMux, routes []route) {
	t.Helper()
	var contract huma.API
	if existing, ok := fixtureAPIs.Load(mux); ok {
		contract = existing.(huma.API)
	} else {
		contract = openapi.New(mux)
		fixtureAPIs.Store(mux, contract)
		t.Cleanup(func() { fixtureAPIs.Delete(mux) })
	}
	for _, candidate := range routes {
		if err := candidate.register(contract); err != nil {
			t.Fatal(err)
		}
	}
}
func fixtureRawRoute(binding openapi.Operation, handler http.HandlerFunc) route {
	return route{operation: binding, register: func(api huma.API) error {
		api.Adapter().Handle(&huma.Operation{Method: binding.Method, Path: binding.Path}, func(ctx huma.Context) { request, writer := humago.Unwrap(ctx); handler(writer, request) })
		return nil
	}}
}

// TestServeMuxRoutingAndCaptures checks method routing, HEAD and escaped captures.
func TestServeMuxRoutingAndCaptures(t *testing.T) {
	routes := []route{}
	for index, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/x/{id}"},
		{Method: "POST", Path: "/api/x/{id}"},
		{Method: "GET", Path: "/api/x/fixed"},
	} {
		routes = append(routes, fixtureRawRoute(operation, func(writer http.ResponseWriter, request *http.Request) {
			if _, err := fmt.Fprintf(writer, "%d:%s", index, request.PathValue("id")); err != nil {
				t.Error(err)
			}
		}))
	}
	handler := &Handler{mux: fixtureMux(routes)}
	for _, scenario := range []struct{ method, path, body, pattern string }{
		{"GET", "/api/x/a", "0:a", "/api/x/{id}"},
		{"HEAD", "/api/x/a", "0:a", "/api/x/{id}"},
		{"POST", "/api/x/a", "1:a", "/api/x/{id}"},
		{"GET", "/api/x/fixed", "2:", "/api/x/fixed"},
		{"GET", "/api/x/%252F", "0:%2F", "/api/x/{id}"},
		{"GET", "/api/x/+", "0:+", "/api/x/{id}"},
	} {
		request := httptest.NewRequest(scenario.method, scenario.path, nil)
		selected, pattern, allow := handler.route(request)
		response := httptest.NewRecorder()
		if selected == nil {
			t.Fatal("missing route", scenario)
		}
		selected.ServeHTTP(response, request)
		if response.Body.String() != scenario.body || pattern != scenario.pattern || allow != "" {
			t.Fatal(scenario, response.Body.String(), pattern, allow)
		}
	}
}

// TestServeMuxMethodRejectionIncludesAllow checks method metadata and safe unmatched labels.
func TestServeMuxMethodRejectionIncludesAllow(t *testing.T) {
	handler := &Handler{mux: fixtureMux([]route{fixtureRawRoute(openapi.Operation{Method: "GET", Path: "/api/x/{id}"}, func(http.ResponseWriter, *http.Request) {})})}
	selected, label, allow := handler.route(httptest.NewRequest("DELETE", "/api/x/private", nil))
	if selected != nil || label != "api.unmatched" || allow != "GET, HEAD" {
		t.Fatal(selected, label, allow)
	}
}

func completeRouter(t *testing.T) (*Handler, *authHandlers, string) {
	t.Helper()
	auth, _, token := authFixture(t)
	var filename, name string
	var sequence int
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		return connection.QueryRowContext(context.Background(), "PRAGMA database_list").Scan(&sequence, &name, &filename)
	}); err != nil {
		t.Fatal(err)
	}
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	notices, err := cfp.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { notices.Close() })
	push, err := delivery.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { push.Close() })
	tasks, err := scheduler.Open(filename)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { tasks.Close() })
	upstream := executor.New(4, 30*time.Second)
	t.Cleanup(upstream.Close)
	handler, err := New(Services{Storage: config.FromProjectRoot(t.TempDir()).WithAuthDbPath(filename), Auth: auth.repository, Cfp: notices, Delivery: push, Scheduler: tasks, Codec: codec, StoragePool: auth.pool, UpstreamPool: upstream, KdfPool: auth.kdfPool}, Options{ContentSecurityPolicy: fixtureCsp})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { handler.Close() })
	return handler, auth, token
}

func TestRouterSecurityCorsCacheAndPrivateLogs(t *testing.T) {
	handler, _, _ := completeRouter(t)
	handler.options.CorsOrigins = []string{"https://allowed.example"}
	handler.options.IsHstsEnabled = true
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	for _, scenario := range []struct {
		method, path, authorization, cookie, cache string
		status                                     int
	}{
		{"GET", "/_next/static/asset.js", "present", "", "private, no-store", 404},
		{"GET", "/page", "present", "", "private, no-store", 404},
		{"GET", "/page", "", "litradar_session=", "private, no-store", 404},
		{"GET", "/page", "", "other=1", "", 404},
		{"GET", "/api/auth/unknown", "present", "", "no-store", 404},
		{"GET", "/api/unknown-private?secret=private-query", "", "", "", 404},
		{"HEAD", "/page///?secret=private-query", "present", "", "private, no-store", 307},
		{"OPTIONS", "/api/auth/me", "present", "", "", 200},
	} {
		request := httptest.NewRequest(scenario.method, scenario.path, nil)
		request.Header.Set("Origin", "https://allowed.example")
		request.Header.Set("Access-Control-Request-Method", "PATCH")
		request.Header.Set("Access-Control-Request-Headers", "authorization,content-type")
		if scenario.authorization != "" {
			request.Header.Set("Authorization", scenario.authorization)
		}
		if scenario.cookie != "" {
			request.Header.Set("Cookie", scenario.cookie)
		}
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		assertRouterSecurityHeaders(t, response, scenario.method, scenario.status, scenario.cache)
	}
	if strings.Contains(logs.String(), "private-query") || strings.Contains(logs.String(), "unknown-private") {
		t.Fatal("request log disclosed private path/query", logs.String())
	}
	if !strings.Contains(logs.String(), `"route":"api.unmatched"`) {
		t.Fatal("missing safe terminal request log", logs.String())
	}
}

// TestServeMuxMatchesMethodAndDecodedPath checks method-specific routing on decoded paths.
func TestServeMuxMatchesMethodAndDecodedPath(t *testing.T) {
	write := func(writer http.ResponseWriter, request *http.Request) {
		_, _ = writer.Write([]byte(request.PathValue("id")))
	}
	handler := &Handler{mux: fixtureMux([]route{fixtureRawRoute(openapi.Operation{Method: "GET", Path: "/x/{id}/fixed/fixed"}, write), fixtureRawRoute(openapi.Operation{Method: "POST", Path: "/x/static/{id}/{tail}"}, write)})}
	for _, scenario := range []struct{ method, path, route, allow string }{
		{"GET", "/x/static/fixed/fixed", "/x/{id}/fixed/fixed", ""},
		{"GET", "/x/other/fixed/fixed", "/x/{id}/fixed/fixed", ""},
		{"GET", "/x/%73tatic/fixed/fixed", "/x/{id}/fixed/fixed", ""},
	} {
		request := httptest.NewRequest(scenario.method, scenario.path, nil)
		_, label, allow := handler.route(request)
		if label != scenario.route || allow != scenario.allow {
			t.Fatal(scenario, label, allow)
		}
	}
}

func TestCompleteRouterMcpAuthenticationAndStreaming(t *testing.T) {
	handler, auth, token := completeRouter(t)
	server := httptest.NewServer(handler)
	defer server.Close()
	client := &http.Client{Timeout: 10 * time.Second}
	session := ""
	post := func(body string) (int, string) {
		t.Helper()
		request, err := http.NewRequest("POST", server.URL+"/mcp", strings.NewReader(body))
		if err != nil {
			t.Fatal(err)
		}
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Accept", "application/json, text/event-stream")
		if session != "" {
			request.Header.Set("Mcp-Session-Id", session)
			request.Header.Set("Mcp-Protocol-Version", "2025-06-18")
		}
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		if value := response.Header.Get("Mcp-Session-Id"); value != "" {
			session = value
		}
		data, err := io.ReadAll(response.Body)
		if err != nil {
			t.Fatal(err)
		}
		if response.Header.Get("Cache-Control") != "private, no-store" || response.Header.Get("X-Request-Id") == "" {
			t.Fatal("MCP bypassed middleware", response.Header)
		}
		return response.StatusCode, string(data)
	}
	status, body := post(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"full-router","version":"1"}}}`)
	if status != 200 || session == "" || !strings.Contains(body, "data: ") {
		t.Fatal(status, body, session)
	}
	status, body = post(`{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}`)
	if status != 200 || !strings.Contains(body, `"name":"search_articles"`) {
		t.Fatal(status, body)
	}
	if _, err := auth.service.RevokeToken(context.Background(), token, domainauth.AuditEvent{Action: "logout", Outcome: "completed", OccurredAt: currentTimestamp()}); err != nil {
		t.Fatal(err)
	}
	status, body = post(`{"jsonrpc":"2.0","id":3,"method":"tools/list","params":{}}`)
	if status != 401 {
		t.Fatal("session bypassed current token", status, body)
	}
}

func TestMiddlewareFlushesBeforeStreamCompletion(t *testing.T) {
	release := make(chan struct{})
	handler := &Handler{options: Options{ContentSecurityPolicy: fixtureCsp}, mux: fixtureMux([]route{fixtureRawRoute(openapi.Operation{Method: "GET", Path: "/stream"}, func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/event-stream")
		_, _ = writer.Write([]byte("data: first\n\n"))
		if err := http.NewResponseController(writer).Flush(); err != nil {
			t.Error(err)
			return
		}
		select {
		case <-release:
		case <-request.Context().Done():
		}
	})})}
	server := httptest.NewServer(handler)
	defer server.Close()
	defer close(release)
	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()
	request, _ := http.NewRequestWithContext(ctx, "GET", server.URL+"/stream", nil)
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		t.Fatal("middleware buffered live stream", err)
	}
	defer response.Body.Close()
	line, err := bufio.NewReader(response.Body).ReadString('\n')
	if err != nil || line != "data: first\n" {
		t.Fatal(line, err)
	}
}

// assertRouterSecurityHeaders checks response, cache, transport policy and preflight headers.
func assertRouterSecurityHeaders(t *testing.T, response *httptest.ResponseRecorder, method string, status int, cache string) {
	t.Helper()
	if response.Code != status || response.Header().Get("Cache-Control") != cache || response.Header().Get("Strict-Transport-Security") != "max-age=31536000" || response.Header().Get("Access-Control-Allow-Origin") != "https://allowed.example" {
		t.Fatal(method, status, cache, response.Code, response.Header())
	}
	if method == "OPTIONS" && (response.Header().Get("Access-Control-Allow-Methods") != "PATCH" || response.Header().Get("Access-Control-Allow-Headers") != "authorization,content-type" || response.Header().Get("Access-Control-Expose-Headers") != "") {
		t.Fatal(response.Header())
	}
}
