package mcpcompat

import (
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"strings"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestExpandedFrozenRustHttpBoundary(t *testing.T) {
	data, err := os.ReadFile("../../../tests/data/migration/mcp-http-boundary-expanded.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Baseline string
		Cases    []struct {
			Name, Method, Body, ResponseBody string
			Headers                          map[string]string
			Status                           int
			ContentType                      *string
			UseSession                       bool
		}
	}
	if err := json.Unmarshal(data, &fixture); err != nil {
		t.Fatal(err)
	}
	if fixture.Baseline != "6bb1220059c82c19a53a1418fc376fc842fea834" {
		t.Fatal("invalid baseline")
	}
	server := mcp.NewServer(&mcp.Implementation{Name: "fixture", Version: "1"}, nil)
	handler := New(server, func(http.ResponseWriter, *http.Request) (Principal, bool) {
		return Principal{UserId: 1, ExpiresAt: time.Now().Add(time.Hour)}, true
	})
	listener := httptest.NewServer(handler)
	defer listener.Close()
	defer handler.Close()
	client := &http.Client{Timeout: 3 * time.Second}
	initial, _ := http.NewRequest("POST", listener.URL, strings.NewReader(`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}`))
	initial.Header.Set("Accept", "application/json, text/event-stream")
	initial.Header.Set("Content-Type", "application/json")
	response, err := client.Do(initial)
	if err != nil {
		t.Fatal(err)
	}
	session := response.Header.Get("Mcp-Session-Id")
	io.Copy(io.Discard, response.Body)
	response.Body.Close()
	if session == "" {
		t.Fatal("session not initialized")
	}
	for _, scenario := range fixture.Cases {
		t.Run(scenario.Name, func(t *testing.T) {
			request, _ := http.NewRequest(scenario.Method, listener.URL, strings.NewReader(scenario.Body))
			for key, value := range scenario.Headers {
				request.Header.Set(key, value)
			}
			if scenario.UseSession {
				request.Header.Set("Mcp-Session-Id", session)
				request.Header.Set("Mcp-Protocol-Version", "2025-06-18")
			}
			response, err := client.Do(request)
			if err != nil {
				t.Fatal(err)
			}
			body, err := io.ReadAll(response.Body)
			response.Body.Close()
			if err != nil || response.StatusCode != scenario.Status || string(body) != scenario.ResponseBody {
				t.Fatalf("got %d %q %v; want %d %q", response.StatusCode, body, err, scenario.Status, scenario.ResponseBody)
			}
			wantType := ""
			if scenario.ContentType != nil {
				wantType = *scenario.ContentType
			}
			if response.Header.Get("Content-Type") != wantType {
				t.Fatalf("content-type %q want %q", response.Header.Get("Content-Type"), wantType)
			}
		})
	}
}
