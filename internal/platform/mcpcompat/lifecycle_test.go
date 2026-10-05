package mcpcompat

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestConcurrentCloseJoinsTheSameActiveSession(t *testing.T) {
	server := mcp.NewServer(&mcp.Implementation{Name: "fixture", Version: "1"}, nil)
	started, release := make(chan struct{}), make(chan struct{})
	var releaseOnce sync.Once
	defer releaseOnce.Do(func() { close(release) })
	server.AddTool(&mcp.Tool{Name: "blocked", InputSchema: map[string]any{"type": "object"}}, func(context.Context, *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		close(started)
		<-release
		return &mcp.CallToolResult{}, nil
	})
	handler := New(server, func(http.ResponseWriter, *http.Request) (Principal, bool) { return Principal{UserId: 1}, true })
	initial := httptest.NewRecorder()
	handler.ServeHTTP(initial, bodyRequest(strings.NewReader(initializeBody)))
	session := initial.Header().Get("Mcp-Session-Id")
	if session == "" {
		t.Fatal("session not initialized")
	}
	request := bodyRequest(strings.NewReader(`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"blocked","arguments":{}}}`))
	request.Header.Set("Mcp-Session-Id", session)
	request.Header.Set("Mcp-Protocol-Version", internalVersion)
	requestDone := make(chan struct{})
	go func() { defer close(requestDone); handler.ServeHTTP(httptest.NewRecorder(), request) }()
	select {
	case <-started:
	case <-time.After(time.Second):
		t.Fatal("tool did not start")
	}
	closed := make(chan error, 2)
	for range 2 {
		go func() { closed <- handler.Close() }()
	}
	select {
	case err := <-closed:
		t.Error("close returned before active session finished", err)
		closed <- err
	case <-time.After(25 * time.Millisecond):
	}
	releaseOnce.Do(func() { close(release) })
	for range 2 {
		select {
		case err := <-closed:
			if err != nil {
				t.Error(err)
			}
		case <-time.After(time.Second):
			t.Fatal("close failed to join")
		}
	}
	select {
	case <-requestDone:
	case <-time.After(time.Second):
		t.Fatal("closed request still running")
	}
	if err := handler.Close(); err != nil {
		t.Fatal(err)
	}
}
