package api

import (
	"context"
	"errors"
	"net/http"
	"net/netip"
	"net/url"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/internal/auth"
	"github.com/QianFuv/LitRadar/internal/mcp"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/platform/mcpcompat"
	"github.com/QianFuv/LitRadar/internal/sources"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// Services borrows migrated repositories, the deployment key and independently bounded executors.
// The host owns their lifetime and drains all outstanding work before closing them.
type Services struct {
	Storage                            config.Config
	Auth                               *authstorage.Repository
	Cfp                                *cfp.Repository
	Delivery                           *delivery.Repository
	Scheduler                          *scheduler.Repository
	Codec                              *secrets.Codec
	StoragePool, UpstreamPool, KdfPool *executor.Pool
}

// Options contains startup-validated security and routing policy, with an optional frontend service.
type Options struct {
	AreCookiesSecure                  bool
	TrustedProxies                    []netip.Prefix
	RateLimit                         settings.RateLimitPolicy
	ProviderProxy                     sources.ProxySelection
	CorsOrigins, McpHosts, McpOrigins []string
	ContentSecurityPolicy             string
	IsHstsEnabled                     bool
	Frontend                          http.Handler
}

// Handler serves the complete REST and stateful MCP surface using borrowed services.
type Handler struct {
	routes   []compiledRoute
	document []byte
	mcp      *mcpcompat.Handler
	options  Options
}

// New composes all 86 concrete operations and verifies their OpenAPI bindings before serving.
func New(services Services, options Options) (*Handler, error) {
	if services.Auth == nil || services.Cfp == nil || services.Delivery == nil || services.Scheduler == nil || services.Codec == nil || services.StoragePool == nil || services.UpstreamPool == nil || services.KdfPool == nil {
		return nil, errors.New("API services must be initialized before router construction")
	}
	if options.ContentSecurityPolicy == "" {
		return nil, errors.New("API content security policy is required")
	}
	settingsRepository := settings.New(services.Auth, services.Codec)
	favoriteRepository := favorites.New(services.Auth)
	sessions := authstorage.NewCnkiSessions(services.Auth, services.Codec)
	service := auth.New(services.Auth, 2)
	authenticator := NewAuthenticator(service, services.StoragePool)
	authHandlers := &authHandlers{service: service, repository: services.Auth, authenticator: authenticator, pool: services.StoragePool, kdfPool: services.KdfPool, limiter: newAuthRateLimiter(options.RateLimit), trustedProxies: slices.Clone(options.TrustedProxies), isCookieSecure: options.AreCookiesSecure}
	providers, err := newArticleProviders(settingsRepository, sessions, options.ProviderProxy)
	if err != nil {
		return nil, err
	}
	groups := [][]route{
		authHandlers.routes(),
		(&indexHandlers{services.Storage, authenticator, services.StoragePool, &weekly.Cache{}}).routes(),
		(&favoriteHandlers{favoriteRepository, services.Storage, authenticator, services.StoragePool}).routes(),
		(&publicHandlers{services.Storage, services.StoragePool, services.Scheduler}).routes(),
		(&cnkiHandlers{sessions: sessions, authenticator: authenticator, pool: services.StoragePool, upstream: services.UpstreamPool, newClient: liveCnkiClient(options.ProviderProxy.ForProvider(sources.ZjlibProviderName))}).routes(),
		(&adminHandlers{auth: authHandlers, storage: services.Storage, scheduler: services.Scheduler, settings: settingsRepository}).routes(),
		(&cfpHandlers{services.Storage, services.Cfp, services.Codec, authenticator, services.StoragePool}).routes(),
		(&articleHandlers{storage: services.Storage, authenticator: authenticator, settings: settingsRepository, sessions: sessions, providers: providers, pool: services.StoragePool, upstream: services.UpstreamPool}).routes(),
		(&trackingHandlers{storage: services.Storage, repository: services.Delivery, favorites: favoriteRepository, settings: settingsRepository, codec: services.Codec, authenticator: authenticator, pool: services.StoragePool}).routes(),
	}
	routes := []route{}
	bindings := []openapi.Operation{}
	for _, group := range groups {
		routes = append(routes, group...)
		for _, route := range group {
			bindings = append(bindings, route.operation)
		}
	}
	document, err := openapi.Generate(bindings)
	if err != nil {
		return nil, err
	}
	server, err := mcp.NewServer(mcp.Services{Storage: services.Storage, Favorites: favoriteRepository, Pool: services.StoragePool})
	if err != nil {
		return nil, err
	}
	options.TrustedProxies = slices.Clone(options.TrustedProxies)
	options.CorsOrigins = slices.Clone(options.CorsOrigins)
	options.McpHosts = slices.Clone(options.McpHosts)
	options.McpOrigins = slices.Clone(options.McpOrigins)
	handler := mcpcompat.NewWithPolicy(server, authenticator.McpAuthorize, mcpcompat.HostOriginPolicy{AllowedHosts: options.McpHosts, AllowedOrigins: options.McpOrigins})
	return &Handler{routes: compileRoutes(routes), document: document, mcp: handler, options: options}, nil
}

// Close closes MCP sessions; borrowed repositories and executors remain owned by the host.
func (handler *Handler) Close() error { return handler.mcp.Close() }

// GenerateOpenAPI verifies the real handler declarations without opening deployment resources.
func GenerateOpenAPI() ([]byte, error) {
	groups := [][]route{
		(&authHandlers{}).routes(), (&indexHandlers{}).routes(), (&favoriteHandlers{}).routes(),
		(&publicHandlers{}).routes(), (&cnkiHandlers{}).routes(), (&adminHandlers{}).routes(),
		(&cfpHandlers{}).routes(), (&articleHandlers{}).routes(), (&trackingHandlers{}).routes(),
	}
	bindings := []openapi.Operation{}
	for _, group := range groups {
		for _, route := range group {
			bindings = append(bindings, route.operation)
		}
	}
	return openapi.Generate(bindings)
}

func (handler *Handler) route(request *http.Request) (http.Handler, string, string) {
	path := request.URL.EscapedPath()
	if path == "/mcp" || strings.HasPrefix(path, "/mcp/") {
		return handler.mcp, "/mcp", ""
	}
	if path == "/docs" || strings.HasPrefix(path, "/docs/") {
		if request.Method != "GET" && request.Method != "HEAD" {
			return nil, "/docs/{*rest}", "GET,HEAD"
		}
		return http.HandlerFunc(openapi.ServeDocs), "/docs/{*rest}", ""
	}
	if path == "/openapi.json" {
		if request.Method != "GET" && request.Method != "HEAD" {
			return nil, path, "GET,HEAD"
		}
		return http.HandlerFunc(func(writer http.ResponseWriter, request *http.Request) {
			writer.Header().Set("Content-Type", "application/json")
			_, _ = writer.Write(handler.document)
		}), path, ""
	}
	pathParts := strings.Split(path, "/")
	var selected *compiledRoute
	best := ""
	for index := range handler.routes {
		candidate := &handler.routes[index]
		score := candidate.score
		if score < best || !candidate.matches(pathParts) {
			continue
		}
		if score > best {
			selected = candidate
			best = score
		}
		if candidate.operation.Method == request.Method || request.Method == "HEAD" && candidate.operation.Method == "GET" {
			selected = candidate
		}
	}
	if selected != nil {
		if selected.operation.Method != request.Method && !(request.Method == "HEAD" && selected.operation.Method == "GET") {
			return nil, selected.operation.Path, selected.allow
		}
		for index, part := range selected.parts {
			if part.isParameter {
				value, _ := url.PathUnescape(pathParts[index])
				request.SetPathValue(part.text, value)
			}
		}
		return selected.handler, selected.operation.Path, ""
	}
	if handler.options.Frontend != nil && !isBackendPath(path) {
		return handler.options.Frontend, unmatchedRoute(path), ""
	}
	return nil, unmatchedRoute(path), ""
}

type routePart struct {
	text        string
	isParameter bool
}

type compiledRoute struct {
	route
	parts []routePart
	score string
	allow string
}

func compileRoutes(routes []route) []compiledRoute {
	methods := make(map[string][]string)
	for _, candidate := range routes {
		path := candidate.operation.Path
		methods[path] = append(methods[path], candidate.operation.Method)
		if candidate.operation.Method == "GET" {
			methods[path] = append(methods[path], "HEAD")
		}
	}
	result := make([]compiledRoute, len(routes))
	for index, candidate := range routes {
		compiled := compiledRoute{route: candidate, allow: strings.Join(methods[candidate.operation.Path], ",")}
		var score strings.Builder
		for _, text := range strings.Split(candidate.operation.Path, "/") {
			part := routePart{text: text, isParameter: strings.HasPrefix(text, "{") && strings.HasSuffix(text, "}")}
			if part.isParameter {
				part.text = text[1 : len(text)-1]
				score.WriteByte('0')
			} else {
				score.WriteByte('1')
			}
			compiled.parts = append(compiled.parts, part)
		}
		compiled.score = score.String()
		result[index] = compiled
	}
	return result
}

func (candidate *compiledRoute) matches(pathParts []string) bool {
	if len(candidate.parts) != len(pathParts) {
		return false
	}
	for index, part := range candidate.parts {
		if part.isParameter {
			if pathParts[index] == "" {
				return false
			}
			if strings.Contains(pathParts[index], "%") {
				if _, err := url.PathUnescape(pathParts[index]); err != nil {
					return false
				}
			}
		} else if part.text != pathParts[index] {
			return false
		}
	}
	return true
}

func unmatchedRoute(path string) string {
	if strings.HasPrefix(path, "/_next/static/") {
		return "static.asset"
	}
	for _, prefix := range []string{"/api", "/mcp", "/health"} {
		if path == prefix || strings.HasPrefix(path, prefix+"/") {
			return prefix[1:] + ".unmatched"
		}
	}
	return "static.frontend"
}

func withRequestId(request *http.Request, id string) *http.Request {
	request = request.Clone(context.WithValue(request.Context(), requestIdKey{}, id))
	request.Header.Del("X-Request-Id")
	request.Header.Set("X-Request-Id", id)
	return request
}
