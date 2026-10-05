package mcp

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
)

type litradarFixture struct {
	handler  *StreamableHTTPHandler
	listener *httptest.Server
	session  string
	client   *http.Client
}

func TestLitRadarUnknownMethodIsSseError(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil)
	response := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":42,"method":"custom/unknown","params":{}}`, "")
	body, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || response.StatusCode != 200 || !strings.Contains(string(body), `"code":-32601,"message":"custom/unknown"`) {
		t.Fatalf("unknown method %d %s %v", response.StatusCode, body, err)
	}
	response = fixture.request(t, "POST", `{"jsonrpc":"2.0","method":"custom/unknown","params":{}}`, "")
	body, err = io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || response.StatusCode != 202 || len(body) != 0 {
		t.Fatalf("unknown notification %d %s %v", response.StatusCode, body, err)
	}
}

func TestLitRadarNumericIdentifiersAndCompleteBody(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil)
	for _, id := range []string{"9007199254740993", "9223372036854775807", "-9223372036854775808"} {
		response := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":`+id+`,"method":"ping"}`, "ignored-on-post")
		body, err := io.ReadAll(response.Body)
		response.Body.Close()
		if err != nil || response.StatusCode != 200 || !strings.Contains(string(body), `"id":`+id+`,"result"`) {
			t.Fatalf("id %s result %d %s %v", id, response.StatusCode, body, err)
		}
	}
	for _, body := range []string{
		`{"jsonrpc":"2.0","id":1,"method":"ping"} {}`,
		`{"jsonrpc":"2.0","id":1,"method":"ping"} trailing`,
	} {
		response := fixture.request(t, "POST", body, "")
		io.Copy(io.Discard, response.Body)
		response.Body.Close()
		if response.StatusCode != 415 {
			t.Fatalf("malformed body accepted: %s status=%d", body, response.StatusCode)
		}
	}
	for _, body := range []string{`{"jsonrpc":"2.0","id":1.5,"method":"ping"}`, `{"jsonrpc":"2.0","id":9223372036854775808,"method":"ping"}`} {
		response := fixture.request(t, "POST", body, "")
		io.Copy(io.Discard, response.Body)
		response.Body.Close()
		if response.StatusCode != 202 {
			t.Fatalf("legacy notification fallback: %d", response.StatusCode)
		}
	}
}

func TestLitRadarApprovedDuplicateIdIsolationAndReuse(t *testing.T) {
	entered := make(chan struct{})
	release := make(chan struct{})
	var releaseOnce sync.Once
	defer releaseOnce.Do(func() { close(release) })
	var executions atomic.Int32
	fixture := newLitRadarFixture(t, true, func(server *Server) {
		server.AddTool(&Tool{Name: "blocked", InputSchema: map[string]any{"type": "object"}}, func(ctx context.Context, request *CallToolRequest) (*CallToolResult, error) {
			if executions.Add(1) == 1 {
				close(entered)
			}
			select {
			case <-release:
			case <-ctx.Done():
				return nil, ctx.Err()
			}
			return &CallToolResult{Content: []Content{&TextContent{Text: "completed"}}}, nil
		})
	})
	first := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"blocked","arguments":{}}}`, "")
	defer first.Body.Close()
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("first request did not enter tool")
	}
	second := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{"name":"blocked","arguments":{}}}`, "")
	body, err := io.ReadAll(second.Body)
	second.Body.Close()
	if err != nil || second.StatusCode != 400 || !strings.Contains(string(body), "duplicate in-flight request ID 20") {
		t.Fatalf("duplicate observation %d %s %v", second.StatusCode, body, err)
	}
	assertPing := func(selected *litradarFixture, id string) {
		t.Helper()
		response := selected.request(t, "POST", `{"jsonrpc":"2.0","id":`+id+`,"method":"ping"}`, "")
		data, err := io.ReadAll(response.Body)
		response.Body.Close()
		if err != nil || response.StatusCode != 200 || !strings.Contains(string(data), `"id":`+id+`,"result":{}`) {
			t.Fatalf("independent/reused ping %d %s %v", response.StatusCode, data, err)
		}
	}
	assertPing(fixture, "21")
	other := &litradarFixture{handler: fixture.handler, listener: fixture.listener, client: fixture.client}
	initial := other.request(t, "POST", `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"other","version":"1"}}}`, "")
	other.session = initial.Header.Get(sessionIDHeader)
	io.Copy(io.Discard, initial.Body)
	initial.Body.Close()
	if other.session == "" || other.session == fixture.session {
		t.Fatal("independent session not created")
	}
	assertPing(other, "20")
	if executions.Load() != 1 {
		t.Fatalf("duplicate executed tool: %d", executions.Load())
	}
	releaseOnce.Do(func() { close(release) })
	body, err = io.ReadAll(first.Body)
	if err != nil || first.StatusCode != 200 || !strings.Contains(string(body), `"id":20,"result"`) || !strings.Contains(string(body), `"text":"completed"`) {
		t.Fatalf("original response lost: %d %s %v", first.StatusCode, body, err)
	}
	assertPing(fixture, "20")
	if executions.Load() != 1 {
		t.Fatal("rejected tool executed after original completion")
	}
}

func newLitRadarFixture(t *testing.T, enabled bool, configure func(*Server), idle ...time.Duration) *litradarFixture {
	t.Helper()
	server := NewServer(&Implementation{Name: "primitive", Version: "1"}, nil)
	if configure != nil {
		configure(server)
	}
	options := &StreamableHTTPOptions{LitRadarCompatibility: enabled}
	if len(idle) > 0 {
		options.SessionTimeout = idle[0]
	}
	handler := NewStreamableHTTPHandler(func(*http.Request) *Server { return server }, options)
	listener := httptest.NewServer(handler)
	fixture := &litradarFixture{handler: handler, listener: listener, client: &http.Client{Timeout: 5 * time.Second}}
	t.Cleanup(func() { handler.closeAll(); listener.Close() })
	response := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"raw","version":"1"}}}`, "")
	fixture.session = response.Header.Get(sessionIDHeader)
	body, err := io.ReadAll(response.Body)
	response.Body.Close()
	if err != nil || response.StatusCode != 200 || fixture.session == "" {
		t.Fatalf("initialize: %d %s %v", response.StatusCode, body, err)
	}
	if enabled && (!strings.HasPrefix(string(body), "data: \nid: 0\nretry: 3000\n\n") || strings.Contains(string(body), "event:")) {
		t.Fatalf("legacy initialize framing: %q", body)
	}
	response = fixture.request(t, "POST", `{"jsonrpc":"2.0","method":"notifications/initialized"}`, "")
	io.Copy(io.Discard, response.Body)
	response.Body.Close()
	if response.StatusCode != 202 {
		t.Fatalf("initialized: %d", response.StatusCode)
	}
	return fixture
}

func (fixture *litradarFixture) request(t *testing.T, method, body, resume string) *http.Response {
	t.Helper()
	request, err := http.NewRequest(method, fixture.listener.URL, strings.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Accept", "application/json, text/event-stream")
	request.Header.Set(protocolVersionHeader, "2025-06-18")
	if fixture.session != "" {
		request.Header.Set(sessionIDHeader, fixture.session)
	}
	if resume != "" {
		request.Header.Set(lastEventIDHeader, resume)
	}
	response, err := fixture.client.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { response.Body.Close() })
	return response
}

func litradarReadEvent(t *testing.T, reader *bufio.Reader) string {
	t.Helper()
	var event strings.Builder
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			t.Fatalf("reading event: %v (%q)", err, event.String())
		}
		event.WriteString(line)
		if line == "\n" {
			return event.String()
		}
	}
}

func (fixture *litradarFixture) connection(t *testing.T) *streamableServerConn {
	t.Helper()
	fixture.handler.mu.Lock()
	defer fixture.handler.mu.Unlock()
	info := fixture.handler.sessions[fixture.session]
	if info == nil {
		t.Fatal("session disappeared")
	}
	return info.transport.connection
}

func TestLitRadarPrimaryShadowsAndReconnection(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil)
	primary := fixture.request(t, "GET", "", "")
	reader := bufio.NewReader(primary.Body)
	if event := litradarReadEvent(t, reader); event != "data: \nid: 0\nretry: 3000\n\n" {
		t.Fatalf("prime %q", event)
	}
	shadows := make([]*http.Response, 33)
	for index := range shadows {
		shadows[index] = fixture.request(t, "GET", "", "")
		litradarReadEvent(t, bufio.NewReader(shadows[index].Body))
	}
	if body, err := io.ReadAll(shadows[0].Body); err != nil || len(body) != 0 {
		t.Fatalf("oldest shadow not evicted: %q %v", body, err)
	}
	connection := fixture.connection(t)
	if err := connection.Write(context.Background(), &jsonrpc.Request{Method: "notifications/test"}); err != nil {
		t.Fatal(err)
	}
	if event := litradarReadEvent(t, reader); !strings.Contains(event, "notifications/test") || !strings.Contains(event, "id: 0\n") {
		t.Fatalf("primary %q", event)
	}
	primary.Body.Close()
	deadline := time.Now().Add(time.Second)
	for {
		connection.mu.Lock()
		common := connection.streams[""]
		connection.mu.Unlock()
		common.mu.Lock()
		detached := common.litradarPrimary == nil
		common.mu.Unlock()
		if detached {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("primary not released")
		}
		time.Sleep(time.Millisecond)
	}
	replacement := fixture.request(t, "GET", "", "0")
	replay := litradarReadEvent(t, bufio.NewReader(replacement.Body))
	if !strings.Contains(replay, "notifications/test") {
		t.Fatalf("inclusive replay: %q", replay)
	}
	for _, shadow := range shadows[1:] {
		shadow.Body.Close()
	}
	deletion := fixture.request(t, "DELETE", "", "")
	deletion.Body.Close()
	if deletion.StatusCode != 202 {
		t.Fatalf("DELETE %d", deletion.StatusCode)
	}
	if body, err := io.ReadAll(replacement.Body); err != nil || len(body) != 0 {
		t.Fatalf("DELETE left stream open: %q %v", body, err)
	}
	deletion = fixture.request(t, "DELETE", "", "")
	deletion.Body.Close()
	if deletion.StatusCode != 202 {
		t.Fatalf("repeat DELETE %d", deletion.StatusCode)
	}
}

func TestLitRadarActiveRequestReplacementFencesOldWriter(t *testing.T) {
	release := make(chan struct{})
	fixture := newLitRadarFixture(t, true, func(server *Server) {
		AddTool(server, &Tool{Name: "hold"}, func(ctx context.Context, request *CallToolRequest, input struct{}) (*CallToolResult, any, error) {
			select {
			case <-release:
			case <-ctx.Done():
				return nil, nil, ctx.Err()
			}
			return &CallToolResult{Content: []Content{&TextContent{Text: "finished"}}}, nil, nil
		})
	})
	original := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hold","arguments":{}}}`, "")
	if event := litradarReadEvent(t, bufio.NewReader(original.Body)); !strings.Contains(event, "id: 0/0") {
		t.Fatalf("request prime: %q", event)
	}
	replacement := fixture.request(t, "GET", "", "0/0")
	if body, err := io.ReadAll(original.Body); err != nil || len(body) != 0 {
		t.Fatalf("old writer retained: %q %v", body, err)
	}
	close(release)
	body, err := io.ReadAll(replacement.Body)
	if err != nil || !strings.Contains(string(body), "finished") || !strings.Contains(string(body), "id: 1/0") {
		t.Fatalf("replacement lost response: %q %v", body, err)
	}
	completed := fixture.request(t, "GET", "", "1/0")
	body, err = io.ReadAll(completed.Body)
	if err != nil || completed.StatusCode != 200 || len(body) != 0 {
		t.Fatalf("completed replay: %d %q %v", completed.StatusCode, body, err)
	}
}

func TestLitRadarDefaultModeCounterexample(t *testing.T) {
	fixture := newLitRadarFixture(t, false, nil)
	response := fixture.request(t, "DELETE", "", "")
	response.Body.Close()
	if response.StatusCode != 204 {
		t.Fatalf("default DELETE changed: %d", response.StatusCode)
	}
	response = fixture.request(t, "DELETE", "", "")
	response.Body.Close()
	if response.StatusCode != 404 {
		t.Fatalf("default missing session changed: %d", response.StatusCode)
	}
}

func TestLitRadarShutdownRejectsNewSessions(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil)
	if err := fixture.handler.CloseLitRadar(); err != nil {
		t.Fatal(err)
	}
	fixture.session = ""
	response := fixture.request(t, "POST", `{}`, "")
	response.Body.Close()
	if response.StatusCode != 503 {
		t.Fatalf("shutdown admitted request: %d", response.StatusCode)
	}
}

func TestLitRadarPendingPostDoesNotPauseIdle(t *testing.T) {
	fixture := newLitRadarFixture(t, true, func(server *Server) {
		AddTool(server, &Tool{Name: "hold"}, func(ctx context.Context, request *CallToolRequest, input struct{}) (*CallToolResult, any, error) {
			<-ctx.Done()
			return nil, nil, ctx.Err()
		})
	}, 150*time.Millisecond)
	response := fixture.request(t, "POST", `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hold","arguments":{}}}`, "")
	started := time.Now()
	body, err := io.ReadAll(response.Body)
	if err != nil || time.Since(started) > time.Second || !strings.Contains(string(body), "retry: 3000") {
		t.Fatalf("open POST suspended idle: %q %v", body, err)
	}
}

func TestLitRadarActivityRefreshesSessionWithoutRefreshingOnMalformedResume(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil, 400*time.Millisecond)
	connection := fixture.connection(t)
	time.Sleep(250 * time.Millisecond)
	if err := connection.Write(context.Background(), &jsonrpc.Request{Method: "notifications/test"}); err != nil {
		t.Fatal(err)
	}
	time.Sleep(250 * time.Millisecond)
	select {
	case <-connection.done:
		t.Fatal("old timer killed refreshed session")
	default:
	}
	valid := fixture.request(t, "GET", "", "0/99999")
	io.Copy(io.Discard, valid.Body)
	valid.Body.Close()
	time.Sleep(250 * time.Millisecond)
	invalid := fixture.request(t, "GET", "", "malformed")
	io.Copy(io.Discard, invalid.Body)
	invalid.Body.Close()
	select {
	case <-connection.done:
	case <-time.After(300 * time.Millisecond):
		t.Fatal("malformed resume refreshed session")
	}
}

func TestLitRadarFrozenRustHttpBoundary(t *testing.T) {
	data, err := os.ReadFile(filepath.Join("..", "..", "..", "tests", "data", "migration", "mcp-http-boundary.json"))
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Baseline string
		Cases    []struct {
			Name, Method, Body string
			Headers            map[string]string
			Status             int
			ResponseBody       string
			ContentType        *string
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.Baseline != "6bb1220059c82c19a53a1418fc376fc842fea834" {
		t.Fatal("unexpected Rust baseline")
	}
	server := NewServer(&Implementation{Name: "primitive", Version: "1"}, nil)
	handler := NewStreamableHTTPHandler(func(*http.Request) *Server { return server }, &StreamableHTTPOptions{LitRadarCompatibility: true, MaxRequestBodyBytes: -1})
	listener := httptest.NewServer(handler)
	defer listener.Close()
	defer handler.closeAll()
	client := &http.Client{Timeout: time.Second}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Name, func(t *testing.T) {
			request, err := http.NewRequest(scenario.Method, listener.URL, strings.NewReader(scenario.Body))
			if err != nil {
				t.Fatal(err)
			}
			for name, value := range scenario.Headers {
				request.Header.Set(name, value)
			}
			response, err := client.Do(request)
			if err != nil {
				t.Fatal(err)
			}
			defer response.Body.Close()
			body, err := io.ReadAll(response.Body)
			if err != nil || response.StatusCode != scenario.Status || string(body) != scenario.ResponseBody {
				t.Fatalf("HTTP boundary: status=%d body=%q error=%v; want %d %q", response.StatusCode, body, err, scenario.Status, scenario.ResponseBody)
			}
			if scenario.ContentType == nil && response.Header.Get("Content-Type") != "" {
				t.Fatalf("introduced content type %q", response.Header.Get("Content-Type"))
			}
		})
	}
}

func TestLitRadarEventStopsWritingToNonreadingTcpPeer(t *testing.T) {
	started, done := make(chan struct{}), make(chan error, 1)
	server := httptest.NewServer(http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		writer.Header().Set("Content-Type", "text/event-stream")
		close(started)
		done <- writeLitRadarEvent(writer, []byte(strings.Repeat("x", 16<<20)), "1", "")
	}))
	defer server.Close()
	connection, err := net.Dial("tcp", server.Listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := connection.(*net.TCPConn).SetReadBuffer(1024); err != nil {
		t.Fatal(err)
	}
	if _, err := fmt.Fprint(connection, "GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"); err != nil {
		t.Fatal(err)
	}
	<-started
	began := time.Now()
	select {
	case err := <-done:
		if err == nil {
			t.Fatal("nonreader unexpectedly consumed oversized frame")
		}
		t.Logf("stalled event write ended after %s: %v", time.Since(began), err)
	case <-time.After(13 * time.Second):
		connection.Close()
		<-done
		t.Fatal("stalled event exceeded its 10s write budget")
	}
}

func TestLitRadarHealthyStreamOutlivesPerWriteDeadline(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil)
	fixture.client.Timeout = 40 * time.Second
	response := fixture.request(t, "GET", "", "")
	reader := bufio.NewReader(response.Body)
	if event := litradarReadEvent(t, reader); !strings.Contains(event, "retry: 3000") {
		t.Fatal(event)
	}
	started := time.Now()
	for range 2 {
		if event := litradarReadEvent(t, reader); event != ":\n\n" {
			t.Fatal("healthy stream lost keepalive", event)
		}
	}
	if time.Since(started) < 10*time.Second {
		t.Fatal("control did not exceed one write deadline")
	}
	if err := fixture.connection(t).Write(context.Background(), &jsonrpc.Request{Method: "notifications/after-keepalive"}); err != nil {
		t.Fatal(err)
	}
	if event := litradarReadEvent(t, reader); !strings.Contains(event, "notifications/after-keepalive") {
		t.Fatal("healthy stream lost later notification", event)
	}
	response.Body.Close()
}

func TestLitRadarNonreadingSessionReleasesItsHttpLease(t *testing.T) {
	fixture := newLitRadarFixture(t, true, nil)
	connection, err := net.Dial("tcp", fixture.listener.Listener.Addr().String())
	if err != nil {
		t.Fatal(err)
	}
	defer connection.Close()
	if err := connection.(*net.TCPConn).SetReadBuffer(1024); err != nil {
		t.Fatal(err)
	}
	connection.SetDeadline(time.Now().Add(20 * time.Second))
	if _, err := fmt.Fprintf(connection, "GET / HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\nMcp-Session-Id: %s\r\nMcp-Protocol-Version: 2025-06-18\r\n\r\n", fixture.session); err != nil {
		t.Fatal(err)
	}
	response, err := http.ReadResponse(bufio.NewReader(connection), &http.Request{Method: "GET"})
	if err != nil {
		t.Fatal(err)
	}
	litradarReadEvent(t, bufio.NewReader(response.Body))
	transport := fixture.connection(t)
	done := make(chan error, 1)
	go func() {
		done <- transport.Write(context.Background(), &jsonrpc.Request{Method: "notifications/large", Params: json.RawMessage(`{"text":"` + strings.Repeat("x", 16<<20) + `"}`)})
	}()
	select {
	case <-done:
	case <-time.After(13 * time.Second):
		connection.Close()
		<-done
		t.Fatal("session event write remained blocked")
	}
	for deadline := time.Now().Add(time.Second); time.Now().Before(deadline); {
		transport.mu.Lock()
		stream := transport.streams[""]
		transport.mu.Unlock()
		stream.mu.Lock()
		isReleased := stream.litradarPrimary == nil
		stream.mu.Unlock()
		if isReleased {
			return
		}
		time.Sleep(time.Millisecond)
	}
	t.Fatal("failed network write left the HTTP lease running")
}
