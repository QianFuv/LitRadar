package api

import (
	"bufio"
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/internal/api/executor"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/tests/migration/testlog"
)

func TestHealthAndStaticSuccessLogsAreQuietButReadinessFailureIsVisible(t *testing.T) {
	handler, _, _ := completeRouter(t)
	handler.options.Frontend = http.HandlerFunc(func(writer http.ResponseWriter, _ *http.Request) { writer.WriteHeader(200) })
	_, finish := testlog.Capture(t)
	for _, route := range []string{"/health/live", "/_next/static/asset.js", "/page", "/health/ready"} {
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, httptest.NewRequest("GET", route, nil))
		expected := 200
		if route == "/health/ready" {
			expected = 503
		}
		if response.Code != expected || response.Header().Get("X-Request-Id") == "" {
			t.Fatal(route, response.Code, response.Header())
		}
	}
	events := testlog.Events(finish(), "http.request.completed")
	if len(events) != 1 {
		t.Fatal(events)
	}
	testlog.Require(t, events[0], map[string]any{"route": "/health/ready", "status": float64(503), "level": "ERROR"})
}

func TestAcceptedHttpConnectionWaitsForDelayedRequestBytes(t *testing.T) {
	handler, _, _ := completeRouter(t)
	server := httptest.NewServer(handler)
	defer server.Close()
	connection, err := net.DialTimeout("tcp", server.Listener.Addr().String(), time.Second)
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := connection.SetDeadline(time.Now().Add(3 * time.Second)); err != nil {
		t.Fatal(err)
	}
	if _, err := io.WriteString(connection, "GET /health/live HTTP/1.1\r\nHost: localhost\r\n"); err != nil {
		t.Fatal(err)
	}
	time.Sleep(50 * time.Millisecond)
	if _, err := io.WriteString(connection, "Connection: close\r\n\r\n"); err != nil {
		t.Fatal(err)
	}
	response, err := http.ReadResponse(bufio.NewReader(connection), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	body, err := io.ReadAll(response.Body)
	if err != nil || response.StatusCode != 200 || string(body) != `{"status":"ok"}` {
		t.Fatal(response.StatusCode, string(body), err)
	}
}

func TestArticleStatusSupportsLoginFreeFallbackAndRejectsExpiredSession(t *testing.T) {
	handlers, _, _, _ := articleFixture(t)
	for _, name := range []string{"zjlib", "fixture"} {
		registerArticleFixture(t, handlers, name, articleFixtureProvider{true, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
			panic("status must not fetch")
		}})
	}
	setArticleOrder(t, handlers, `{"default":["zjlib","fixture"],"catalogs":{}}`)
	status, failure := handlers.status(context.Background(), articleLocatorFixture(), 1, "fixture")
	if failure != nil || !status.Fulltext.Available || status.Fulltext.RequiresLogin || status.Fulltext.Message != nil {
		t.Fatal(status, failure)
	}
	setArticleOrder(t, handlers, `{"default":["zjlib"],"catalogs":{}}`)
	if _, err := handlers.sessions.Upsert(context.Background(), 1, json.RawMessage(`{"bff_user_token":"header.eyJleHAiOjF9.signature"}`), "active", nil); err != nil {
		t.Fatal(err)
	}
	status, failure = handlers.status(context.Background(), articleLocatorFixture(), 1, "fixture")
	if failure != nil || status.Fulltext.Available || !status.Fulltext.RequiresLogin || status.Fulltext.Message == nil {
		t.Fatal(status, failure)
	}
}

func TestRealAuthKdfPoolBoundsTwoAndDoesNotBlockStorageOrUpstream(t *testing.T) {
	articles, handlers, router, token := articleFixture(t)
	entered, finished := make(chan struct{}, 3), make(chan *apiError, 3)
	release := make(chan struct{})
	isReleased := false
	started, joined := 0, 0
	defer func() {
		if !isReleased {
			close(release)
		}
		for joined < started {
			select {
			case err := <-finished:
				joined++
				if err != nil {
					t.Error("KDF worker failed", err)
				}
			case <-time.After(5 * time.Second):
				t.Error("KDF worker did not drain")
				return
			}
		}
	}()
	start := func() {
		started++
		go func() {
			_, err := runAuth(httptest.NewRequest("POST", "/api/auth/login", nil), handlers.kdfPool, func() (bool, error) { entered <- struct{}{}; <-release; return true, nil })
			finished <- err
		}()
	}
	for range 2 {
		start()
	}
	for range 2 {
		select {
		case <-entered:
		case <-time.After(2 * time.Second):
			t.Fatal("KDF did not admit both slots")
		}
	}
	start()
	select {
	case <-entered:
		t.Fatal("third KDF closure entered while both slots were occupied")
	case <-time.After(50 * time.Millisecond):
	}
	if response := authRequest(router, "GET", "/api/auth/me", "", token); response.Code != 200 {
		t.Fatal("storage-only auth blocked", response.Body.String())
	}
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if result, err := executor.Run(ctx, articles.upstream, func() (string, error) { return "independent", nil }); err != nil || result != "independent" {
		t.Fatal(result, err)
	}
	close(release)
	isReleased = true
	for joined < started {
		select {
		case err := <-finished:
			joined++
			if err != nil {
				t.Fatal(err)
			}
		case <-time.After(5 * time.Second):
			t.Fatal("queued closure did not resume")
		}
	}
	handlers.kdfPool.Close()
	if response := authRequest(router, "POST", "/api/auth/login", `{"username":"api_fixture","password":"correct password long"}`, ""); response.Code != 503 {
		t.Fatal("real login bypassed closed KDF pool", response.Code)
	}
	if response := authRequest(router, "GET", "/api/auth/me", "", token); response.Code != 200 {
		t.Fatal("closing KDF broke storage-only auth", response.Code)
	}
}
