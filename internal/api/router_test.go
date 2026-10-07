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
	"testing"
	"time"

	domainauth "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
)

const fixtureCsp = "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"

func TestRouteCompatibilityOrderingAndCaptures(t *testing.T) {
	routes := []route{}
	for index, operation := range []openapi.Operation{
		{Method: "GET", Path: "/api/x/{id}"},
		{Method: "HEAD", Path: "/api/x/{id}"},
		{Method: "GET", Path: "/api/x/{id}"},
		{Method: "POST", Path: "/api/x/{name}"},
		{Method: "POST", Path: "/api/x/fixed"},
		{Method: "GET", Path: "/api/pair/{id}/{id}"},
	} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) { fmt.Fprint(writer, index) }})
	}
	handler := &Handler{routes: compileRoutes(routes), options: Options{Frontend: http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) { fmt.Fprint(writer, "frontend") })}}
	for _, scenario := range []struct {
		method, path, selected, label, allow, id, name string
	}{
		{"GET", "/api/x/a", "2", "/api/x/{id}", "", "a", ""},
		{"HEAD", "/api/x/a", "2", "/api/x/{id}", "", "a", ""},
		{"POST", "/api/x/a", "3", "/api/x/{name}", "", "", "a"},
		{"DELETE", "/api/x/a", "", "/api/x/{id}", "GET,HEAD,HEAD,GET,HEAD", "", ""},
		{"GET", "/api/x/fixed", "", "/api/x/fixed", "POST", "", ""},
		{"GET", "/api/x/%2F", "2", "/api/x/{id}", "", "/", ""},
		{"GET", "/api/x/%252F", "2", "/api/x/{id}", "", "%2F", ""},
		{"GET", "/api/x/+", "2", "/api/x/{id}", "", "+", ""},
		{"GET", "/api/x/%FF", "2", "/api/x/{id}", "", string([]byte{0xff}), ""},
		{"GET", "/api/x/", "", "api.unmatched", "", "", ""},
		{"GET", "/api/x/a/", "", "api.unmatched", "", "", ""},
		{"GET", "/api/pair/first/last", "5", "/api/pair/{id}/{id}", "", "last", ""},
		{"GET", "/page", "frontend", "static.frontend", "", "", ""},
	} {
		request := httptest.NewRequest(scenario.method, scenario.path, nil)
		request.SetPathValue("prior", "preserved")
		selected, label, allow := handler.route(request)
		response := httptest.NewRecorder()
		if selected != nil {
			selected.ServeHTTP(response, request)
		}
		if response.Body.String() != scenario.selected || label != scenario.label || allow != scenario.allow || request.PathValue("id") != scenario.id || request.PathValue("name") != scenario.name || request.PathValue("prior") != "preserved" {
			t.Fatalf("%+v: selected=%q label=%q allow=%q id=%q name=%q", scenario, response.Body.String(), label, allow, request.PathValue("id"), request.PathValue("name"))
		}
	}
	assertLastExplicitHeadDeclaration(t, handler, routes)

}

func TestRouteMalformedCapturesAreRejected(t *testing.T) {
	for _, path := range []string{"/x/%", "/x/%2", "/x/%GG"} {
		candidate := compileRoutes([]route{{openapi.Operation{Path: "/x/{id}"}, nil}})[0]
		if candidate.matches(strings.Split(path, "/")) {
			t.Fatal("malformed percent escape matched", path)
		}
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
	handler.options.Frontend = http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Cache-Control", "handler-policy")
		writer.WriteHeader(200)
		_, _ = writer.Write([]byte("frontend"))
	})
	var logs bytes.Buffer
	previous := slog.Default()
	slog.SetDefault(slog.New(slog.NewJSONHandler(&logs, nil)))
	defer slog.SetDefault(previous)
	for _, scenario := range []struct {
		method, path, authorization, cookie, cache string
		status                                     int
	}{
		{"GET", "/_next/static/asset.js", "present", "", "public, max-age=31536000, immutable", 200},
		{"GET", "/page", "present", "", "private, no-store", 200},
		{"GET", "/page", "", "litradar_session=", "private, no-store", 200},
		{"GET", "/page", "", "other=1", "no-cache", 200},
		{"GET", "/api/auth/unknown", "present", "", "no-store", 404},
		{"GET", "/api/unknown-private?secret=private-query", "", "", "", 404},
		{"HEAD", "/page///?secret=private-query", "present", "", "", 308},
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

func TestRouteStaticPriorityBacktrackingAndRawPath(t *testing.T) {
	write := func(writer http.ResponseWriter, request *http.Request) {
		_, _ = writer.Write([]byte(request.PathValue("id")))
	}
	handler := &Handler{routes: compileRoutes([]route{{openapi.Operation{Method: "GET", Path: "/x/{id}/fixed/fixed"}, write}, {openapi.Operation{Method: "POST", Path: "/x/static/{id}/{tail}"}, write}})}
	for _, scenario := range []struct{ method, path, route, allow string }{
		{"GET", "/x/static/fixed/fixed", "/x/static/{id}/{tail}", "POST"},
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
	handler := &Handler{options: Options{ContentSecurityPolicy: fixtureCsp}, routes: compileRoutes([]route{{openapi.Operation{Method: "GET", Path: "/stream"}, func(writer http.ResponseWriter, request *http.Request) {
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
	}}})}
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

// assertLastExplicitHeadDeclaration checks declaration-order selection of an explicit HEAD handler.
func assertLastExplicitHeadDeclaration(t *testing.T, handler *Handler, routes []route) {
	t.Helper()
	handler.routes = compileRoutes(routes[:2])
	selected, _, _ := handler.route(httptest.NewRequest("HEAD", "/api/x/a", nil))
	response := httptest.NewRecorder()
	selected.ServeHTTP(response, httptest.NewRequest("HEAD", "/api/x/a", nil))
	if response.Body.String() != "1" {
		t.Fatal("last explicit HEAD declaration lost", response.Body.String())
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
