// Package mcpcompat adapts LitRadar's legacy stateful contract through the official SDK's public boundaries.
package mcpcompat

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/modelcontextprotocol/go-sdk/auth"
	"github.com/modelcontextprotocol/go-sdk/jsonrpc"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

const internalVersion = "2025-06-18"
const principalExtra = "litradar.verifiedPrincipal"
const versionExtra = "litradar.initializeVersion"

// Principal comes only from the application's authentication result for this HTTP request.
type Principal struct {
	UserId    identity.Id
	ExpiresAt time.Time
}

type principalKey struct{}
type versionKey struct{}

// Authorize authenticates each request and owns the public error response when authentication fails.
type Authorize func(http.ResponseWriter, *http.Request) (Principal, bool)

// Handler preserves SDK-owned sessions while attaching fresh authenticated identity to every request.
type Handler struct {
	transport *mcp.StreamableHTTPHandler
	next      http.Handler
	authorize Authorize
	policy    HostOriginPolicy
}

// New installs legacy initialization adaptation once on an application-owned server.
func New(server *mcp.Server, authorize Authorize) *Handler {
	return NewWithPolicy(server, authorize, HostOriginPolicy{AllowedHosts: []string{"localhost", "127.0.0.1", "::1"}})
}

// NewWithPolicy installs explicit Host/Origin checks before request body decoding.
func NewWithPolicy(server *mcp.Server, authorize Authorize, policy HostOriginPolicy) *Handler {
	server.AddReceivingMiddleware(func(next mcp.MethodHandler) mcp.MethodHandler {
		return func(ctx context.Context, method string, request mcp.Request) (mcp.Result, error) {
			if method != "initialize" {
				return next(ctx, method, request)
			}
			original, ok := request.(*mcp.ServerRequest[*mcp.InitializeParams])
			if !ok || original.Params == nil {
				return next(ctx, method, request)
			}
			if original.Params.Capabilities == nil || original.Params.ClientInfo == nil {
				return nil, &jsonrpc.Error{Code: jsonrpc.CodeMethodNotFound, Message: "initialize"}
			}
			copyRequest := *original
			copyParams := *original.Params
			copyParams.ProtocolVersion = internalVersion
			copyRequest.Params = &copyParams
			var result mcp.Result = &mcp.InitializeResult{}
			if original.Session.InitializeParams() == nil {
				var err error
				result, err = next(ctx, method, &copyRequest)
				if err != nil {
					return result, err
				}
			}
			initialize := result.(*mcp.InitializeResult)
			copyResult := *initialize
			copyResult.ProtocolVersion = "2025-11-25"
			if original.Extra != nil && original.Extra.TokenInfo != nil {
				if version, ok := original.Extra.TokenInfo.Extra[versionExtra].(string); ok && knownVersion(version) {
					copyResult.ProtocolVersion = version
				}
			}
			copyResult.Capabilities = &mcp.ServerCapabilities{Tools: &mcp.ToolCapabilities{}}
			copyResult.ServerInfo = &mcp.Implementation{Name: "rmcp", Version: "2.1.0"}
			copyResult.Instructions = "Use LitRadar tools to query indexed papers and favorites."
			return &copyResult, nil
		}
	})
	transport := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return server }, &mcp.StreamableHTTPOptions{LitRadarCompatibility: true, MaxRequestBodyBytes: -1, DisableLocalhostProtection: true})
	verified := auth.RequireBearerToken(func(ctx context.Context, _ string, _ *http.Request) (*auth.TokenInfo, error) {
		principal, ok := ctx.Value(principalKey{}).(Principal)
		if !ok {
			return nil, auth.ErrInvalidToken
		}
		return &auth.TokenInfo{Expiration: principal.ExpiresAt, Extra: map[string]any{principalExtra: principal, versionExtra: ctx.Value(versionKey{})}}, nil
	}, &auth.RequireBearerTokenOptions{AllowMissingExpiration: true})(transport)
	return &Handler{transport: transport, next: verified, authorize: authorize, policy: HostOriginPolicy{AllowedHosts: append([]string(nil), policy.AllowedHosts...), AllowedOrigins: append([]string(nil), policy.AllowedOrigins...)}}
}

// PrincipalFor reads current-request identity from SDK-owned extra context, never client metadata.
func PrincipalFor(request mcp.Request) (Principal, bool) {
	extra := request.GetExtra()
	if extra == nil || extra.TokenInfo == nil {
		return Principal{}, false
	}
	principal, ok := extra.TokenInfo.Extra[principalExtra].(Principal)
	return principal, ok
}

func knownVersion(version string) bool {
	switch version {
	case "2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25", "2026-07-28":
		return true
	}
	return false
}

// ServeHTTP applies external version consistency before adapting transport labels or metadata.
func (handler *Handler) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	principal, ok := handler.authorize(writer, request)
	if !ok {
		return
	}
	if !handler.policy.validate(writer, request) {
		return
	}
	prepared := request.Clone(context.WithValue(request.Context(), principalKey{}, principal))
	prepared.Header.Set("Authorization", "Bearer verified-by-litradar")
	version := request.Header.Get("MCP-Protocol-Version")
	if knownVersion(version) {
		prepared.Header.Set("MCP-Protocol-Version", internalVersion)
	}
	if request.Method == http.MethodPost && strings.Contains(request.Header.Get("Accept"), "application/json") && strings.Contains(request.Header.Get("Accept"), "text/event-stream") && strings.HasPrefix(request.Header.Get("Content-Type"), "application/json") {
		body, err := io.ReadAll(request.Body)
		if err != nil {
			writer.Header()["Content-Type"] = nil
			writer.WriteHeader(500)
			io.WriteString(writer, "Failed to read request body: "+err.Error())
			return
		}
		if err := mcp.ValidateLitRadarMessage(body); err != nil {
			writer.Header()["Content-Type"] = nil
			writer.WriteHeader(415)
			io.WriteString(writer, "fail to deserialize request body "+err.Error())
			return
		}
		var envelope map[string]json.RawMessage
		if json.Unmarshal(body, &envelope) == nil && envelope != nil {
			var method string
			_ = json.Unmarshal(envelope["method"], &method)
			var params map[string]json.RawMessage
			if json.Unmarshal(envelope["params"], &params) == nil && params != nil {
				if method == "initialize" {
					var requested string
					if json.Unmarshal(params["protocolVersion"], &requested) == nil {
						prepared = prepared.WithContext(mcp.WithLitRadarVersions(prepared.Context(), version, requested))
						prepared = prepared.WithContext(context.WithValue(prepared.Context(), versionKey{}, requested))
						params["protocolVersion"] = json.RawMessage(`"` + internalVersion + `"`)
						if version != "" && len(request.Header.Values("Mcp-Session-Id")) == 0 {
							prepared.Header.Set("MCP-Protocol-Version", internalVersion)
						}
					}
				}
				var metadata map[string]json.RawMessage
				if json.Unmarshal(params["_meta"], &metadata) == nil && metadata != nil {
					delete(metadata, mcp.MetaKeyProtocolVersion)
					params["_meta"], _ = json.Marshal(metadata)
				}
				envelope["params"], _ = json.Marshal(params)
				body, _ = json.Marshal(envelope)
			}
		}
		prepared.Body = io.NopCloser(bytes.NewReader(body))
		prepared.ContentLength = int64(len(body))
	}
	handler.next.ServeHTTP(writer, prepared)
}

// Close stops new MCP requests and drains the SDK-owned sessions.
func (handler *Handler) Close() error { return handler.transport.CloseLitRadar() }
