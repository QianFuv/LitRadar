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

const testBodyLimit = 4 << 20
const initializeBody = `{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}}`

type countedBody struct {
	io.Reader
	count int
}

func (body *countedBody) Read(buffer []byte) (int, error) {
	count, err := body.Reader.Read(buffer)
	body.count += count
	return count, err
}
func (*countedBody) Close() error { return nil }

func bodyHandler(t *testing.T) *Handler {
	t.Helper()
	handler := NewWithPolicy(mcp.NewServer(&mcp.Implementation{Name: "fixture", Version: "1"}, nil), func(writer http.ResponseWriter, request *http.Request) (Principal, bool) {
		if request.Header.Get("Authorization") == "reject" {
			writer.WriteHeader(401)
			return Principal{}, false
		}
		return Principal{UserId: 1, ExpiresAt: time.Now().Add(time.Hour)}, true
	}, HostOriginPolicy{AllowedHosts: []string{"localhost", "127.0.0.1"}, AllowedOrigins: []string{"http://localhost"}})
	t.Cleanup(func() { handler.Close() })
	return handler
}

func bodyRequest(body io.Reader) *http.Request {
	request := httptest.NewRequest("POST", "http://localhost/mcp", body)
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Accept", "application/json, text/event-stream")
	return request
}

func TestBodyBudgetBoundsFirstReadAndKeepsHeaderPrecedence(t *testing.T) {
	handler := bodyHandler(t)
	for _, scenario := range []struct {
		name              string
		change            func(*http.Request)
		status, readLimit int
	}{
		{"oversize", func(*http.Request) {}, 413, testBodyLimit + 1},
		{"authentication", func(request *http.Request) { request.Header.Set("Authorization", "reject"); request.Host = "evil.test" }, 401, 0},
		{"host", func(request *http.Request) { request.Host = "evil.test" }, 403, 0},
		{"origin", func(request *http.Request) { request.Header.Set("Origin", "http://evil.test") }, 403, 0},
		{"accept", func(request *http.Request) { request.Header.Del("Accept") }, 406, 0},
		{"content type", func(request *http.Request) { request.Header.Set("Content-Type", "text/plain") }, 415, 0},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			body := &countedBody{Reader: strings.NewReader(initializeBody + strings.Repeat(" ", 2*testBodyLimit))}
			request := bodyRequest(body)
			request.ContentLength = -1
			scenario.change(request)
			writer := httptest.NewRecorder()
			handler.ServeHTTP(writer, request)
			if writer.Code != scenario.status || body.count > scenario.readLimit {
				t.Fatalf("status=%d bytes read=%d; want status=%d at most=%d", writer.Code, body.count, scenario.status, scenario.readLimit)
			}
		})
	}
}

func TestBodyBudgetAcceptsExactLimitAndNormalizedExpansion(t *testing.T) {
	handler := bodyHandler(t)
	for _, body := range []string{
		initializeBody + strings.Repeat(" ", testBodyLimit-len(initializeBody)),
		strings.Replace(initializeBody, `"capabilities":{}`, `"capabilities":{},"_meta":{"io.modelcontextprotocol/protocolVersion":"ignored","retained":"`+strings.Repeat("<\u2028\\u003e", testBodyLimit/20)+`"}`, 1),
	} {
		body += strings.Repeat(" ", testBodyLimit-len(body))
		writer := httptest.NewRecorder()
		handler.ServeHTTP(writer, bodyRequest(strings.NewReader(body)))
		if writer.Code != 200 {
			t.Fatalf("bounded input rejected after normalization: %d %s", writer.Code, writer.Body.String())
		}
	}
}

func TestBodyBudgetRejectsChunkedOversize(t *testing.T) {
	handler := bodyHandler(t)
	server := httptest.NewServer(handler)
	defer server.Close()
	request, _ := http.NewRequest("POST", server.URL, io.NopCloser(strings.NewReader(initializeBody+strings.Repeat(" ", testBodyLimit-len(initializeBody)+1))))
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Accept", "application/json, text/event-stream")
	client := &http.Client{Timeout: 5 * time.Second}
	response, err := client.Do(request)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	if response.StatusCode != 413 {
		t.Fatal("chunked body bypassed budget", response.StatusCode)
	}
}

func TestUnchangedBodyKeepsOriginalRepresentation(t *testing.T) {
	handler := bodyHandler(t)
	body := `{ "jsonrpc": "2.0", "id": 5, "method": "tools/call", "params":{"_meta":{"retained":true},"arguments":{},"name":"inspect"} }`
	handler.next = http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
		actual, err := io.ReadAll(request.Body)
		if err != nil || string(actual) != body {
			t.Errorf("unchanged request was reencoded: %s %v", actual, err)
		}
		writer.WriteHeader(200)
	})
	handler.ServeHTTP(httptest.NewRecorder(), bodyRequest(strings.NewReader(body)))
}

func TestNormalizationPreservesDuplicateAndInvalidParameterBytes(t *testing.T) {
	handler := bodyHandler(t)
	for _, scenario := range []struct{ body, want string }{
		{`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"inspect","name":null,"arguments":{}}}`, `{"arguments":{},"name":null}`},
		{`{"jsonrpc":"2.0","id":1,"method":"tools/call","params": { "name": 3, "arguments": {} }}`, `{"arguments":{},"name":3}`},
		{`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{},"name":"old"},"params":{"arguments":{},"name":"new"}}`, `{"arguments":{},"name":"new"}`},
	} {
		handler.next = http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
			body, err := io.ReadAll(request.Body)
			if err != nil {
				t.Fatal(err)
			}
			fields, hasDuplicates := objectFields(body)
			if hasDuplicates || string(fields["params"]) != scenario.want {
				t.Errorf("legacy normalization changed: %s", body)
			}
			writer.WriteHeader(200)
		})
		handler.ServeHTTP(httptest.NewRecorder(), bodyRequest(strings.NewReader(scenario.body)))
	}
}
