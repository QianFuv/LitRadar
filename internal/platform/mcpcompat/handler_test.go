package mcpcompat

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func TestLegacyVersionsCurrentIdentityAndTextResults(t *testing.T) {
	server := mcp.NewServer(&mcp.Implementation{Name: "private-fixture", Version: "1"}, nil)
	var authorizeCalls atomic.Int32
	server.AddTool(&mcp.Tool{Name: "inspect", InputSchema: map[string]any{"type": "object"}}, func(_ context.Context, request *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		principal, ok := PrincipalFor(request)
		if !ok {
			return nil, &jsonrpc.Error{Code: -32603, Message: "missing trusted identity"}
		}
		payload, _ := json.Marshal(map[string]any{"user_id": principal.UserId, "internal_version": request.Session.InitializeParams().ProtocolVersion, "metadata": request.Params.Meta})
		return &mcp.CallToolResult{Content: []mcp.Content{&mcp.TextContent{Text: string(payload)}}}, nil
	})
	server.AddTool(&mcp.Tool{Name: "failure", InputSchema: map[string]any{"type": "object"}}, func(context.Context, *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
		return nil, &jsonrpc.Error{Code: -32602, Message: "fixture invalid input"}
	})
	handler := New(server, func(writer http.ResponseWriter, request *http.Request) (Principal, bool) {
		authorizeCalls.Add(1)
		principal := Principal{}
		switch request.Header.Get("Authorization") {
		case "Bearer alice":
			principal.UserId = 9007199254740993
		case "Bearer bob":
			principal.UserId = 99
		default:
			writer.WriteHeader(401)
			return Principal{}, false
		}
		return principal, true
	})
	listener := httptest.NewServer(handler)
	defer listener.Close()
	defer handler.Close()
	client := &http.Client{Timeout: 3 * time.Second}
	post := func(token, session, version, body string) (int, string, string) {
		request, _ := http.NewRequest("POST", listener.URL, strings.NewReader(body))
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("Accept", "application/json, text/event-stream")
		request.Header.Set("Content-Type", "application/json")
		if session != "" {
			request.Header.Set("MCP-Session-Id", session)
		}
		if version != "" {
			request.Header.Set("MCP-Protocol-Version", version)
		}
		response, err := client.Do(request)
		if err != nil {
			t.Fatal(err)
		}
		defer response.Body.Close()
		data, err := io.ReadAll(response.Body)
		if err != nil {
			t.Fatal(err)
		}
		return response.StatusCode, response.Header.Get("MCP-Session-Id"), string(data)
	}
	for _, version := range []string{"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28", "future-unknown"} {
		t.Run(version, func(t *testing.T) {
			status, session, wire := post("alice", "", version, `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"`+version+`","capabilities":{},"clientInfo":{"name":"raw","version":"1"},"_meta":{"retained":"yes"}}}`)
			if status != 200 || session == "" {
				t.Fatalf("initialize %d %s", status, wire)
			}
			var initialized struct {
				Result struct {
					ProtocolVersion string
					Capabilities    map[string]any
					ServerInfo      map[string]any
				}
			}
			decodeLastData(t, wire, &initialized)
			want := version
			if !knownVersion(want) {
				want = "2025-11-25"
			}
			if initialized.Result.ProtocolVersion != want || len(initialized.Result.Capabilities) != 1 || initialized.Result.ServerInfo["name"] != "rmcp" {
				t.Fatalf("outward initialization %s", wire)
			}
			status, _, wire = post("bob", session, want, `{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"inspect","arguments":{},"_meta":{"user_id":"attacker","retained":"yes","io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}`)
			if status != 200 {
				t.Fatalf("current user call %d %s", status, wire)
			}
			var result struct {
				Result struct {
					Content           []struct{ Type, Text string }
					StructuredContent any
				}
			}
			decodeLastData(t, wire, &result)
			if len(result.Result.Content) != 1 || result.Result.Content[0].Type != "text" || result.Result.StructuredContent != nil {
				t.Fatalf("text wrapping %s", wire)
			}
			var payload struct {
				UserId          string `json:"user_id"`
				InternalVersion string `json:"internal_version"`
				Metadata        map[string]any
			}
			if err := json.Unmarshal([]byte(result.Result.Content[0].Text), &payload); err != nil {
				t.Fatal(err)
			}
			if payload.UserId != "99" || payload.InternalVersion != internalVersion || payload.Metadata["retained"] != "yes" {
				t.Fatalf("identity/state/metadata %s", result.Result.Content[0].Text)
			}
			status, _, wire = post("alice", session, want, `{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"failure","arguments":{}}}`)
			var failure struct {
				Error struct {
					Code    int
					Message string
				}
			}
			decodeLastData(t, wire, &failure)
			if status != 200 || failure.Error.Code != -32602 || failure.Error.Message != "fixture invalid input" {
				t.Fatalf("JSON-RPC error became tool success: %s", wire)
			}
			status, _, _ = post("revoked", session, want, `{"jsonrpc":"2.0","id":4,"method":"ping"}`)
			if status != 401 {
				t.Fatal("session bypassed current credential validation")
			}
			status, responseSession, repeated := post("alice", session, want, `{"jsonrpc":"2.0","id":5,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"again","version":"2"}}}`)
			if status != 200 || responseSession != "" || !strings.Contains(repeated, `"protocolVersion":"2024-11-05"`) || !strings.Contains(repeated, "id: 0/") {
				t.Fatalf("repeated initialize changed session/framing: %d %s %s", status, responseSession, repeated)
			}
		})
	}
	if authorizeCalls.Load() != 30 {
		t.Fatalf("per-request authorization %d", authorizeCalls.Load())
	}
	status, _, wire := post("alice", "", "2025-06-18", `{"jsonrpc":"2.0","id":9007199254740993,"method":"initialize","params":{"protocolVersion":"2026-07-28","capabilities":{},"clientInfo":{"name":"raw","version":"1"}}}`)
	if status != 400 || !strings.Contains(wire, "does not match") || !strings.Contains(wire, "9007199254740993") {
		t.Fatalf("external consistency/id precision: %d %s", status, wire)
	}
}

func decodeLastData(t *testing.T, wire string, target any) {
	t.Helper()
	for _, line := range strings.Split(wire, "\n") {
		if strings.HasPrefix(line, "data: {") {
			if err := json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), target); err != nil {
				t.Fatal(err)
			}
			return
		}
	}
	t.Fatalf("missing JSON SSE payload: %s", wire)
}
