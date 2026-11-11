package api

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/netip"
	"slices"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/mcp"
	"github.com/QianFuv/LitRadar/apps/server/internal/openapi"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/executor"
	"github.com/QianFuv/LitRadar/apps/server/internal/platform/mcpcompat"
	"github.com/QianFuv/LitRadar/apps/server/internal/sources"
	authstorage "github.com/QianFuv/LitRadar/apps/server/internal/storage/auth"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/config"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/favorites"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/weekly"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
	"github.com/danielgtaylor/huma/v2"
)

// Services borrows migrated repositories, the deployment key and independently bounded executors.
// The host owns their lifetime and drains all outstanding work before closing them.
type Services struct {
	Telemetry                          *telemetry.Collector
	Storage                            config.Config
	Auth                               *authstorage.Repository
	Cfp                                *cfp.Repository
	Delivery                           *delivery.Repository
	Scheduler                          *scheduler.Repository
	Codec                              *secrets.Codec
	StoragePool, UpstreamPool, KdfPool *executor.Pool
}

// Options contains startup-validated security and routing policy.
type Options struct {
	AreCookiesSecure                  bool
	IsDevelopment                     bool
	TrustedProxies                    []netip.Prefix
	RateLimit                         settings.RateLimitPolicy
	ProviderProxy                     sources.ProxySelection
	CorsOrigins, McpHosts, McpOrigins []string
	ContentSecurityPolicy             string
	IsHstsEnabled                     bool
}

// Handler serves the complete REST and stateful MCP surface using borrowed services.
type Handler struct {
	telemetry *telemetry.Collector
	mux       *http.ServeMux
	mcp       *mcpcompat.Handler
	options   Options
}

// New composes concrete operations and verifies their OpenAPI bindings before serving.
func New(services Services, options Options) (*Handler, error) {
	if err := validateAPIServices(services); err != nil {
		return nil, err
	}
	if options.ContentSecurityPolicy == "" {
		return nil, errors.New("API content security policy is required")
	}
	settingsRepository := settings.New(services.Auth, services.Codec, options.IsDevelopment)
	favoriteRepository := favorites.New(services.Auth)
	sessions := authstorage.NewCnkiSessions(services.Auth, services.Codec)
	service := auth.New(services.Auth, 2)
	authenticator := NewAuthenticator(service, services.StoragePool)
	authHandlers := &authHandlers{service: service, repository: services.Auth, authenticator: authenticator, pool: services.StoragePool, kdfPool: services.KdfPool, limiter: newAuthRateLimiter(options.RateLimit), trustedProxies: slices.Clone(options.TrustedProxies), isCookieSecure: options.AreCookiesSecure}
	authHandlers.oidcPool = services.UpstreamPool
	authHandlers.oidc = auth.NewOidc(service, settingsRepository, services.Codec, options.IsDevelopment)
	providers, err := newArticleProviders(settingsRepository, sessions, options.ProviderProxy)
	if err != nil {
		return nil, err
	}
	groups := [][]route{
		authHandlers.routes(),
		(&indexHandlers{services.Storage, authenticator, services.StoragePool, &weekly.Cache{}}).routes(),
		(&favoriteHandlers{favoriteRepository, services.Storage, authenticator, services.StoragePool}).routes(),
		(&favoriteHandlers{favoriteRepository, services.Storage, authenticator, services.StoragePool}).graphRoutes(),
		(&publicHandlers{services.Storage, services.StoragePool, services.Scheduler}).routes(),
		(&cnkiHandlers{sessions: sessions, authenticator: authenticator, pool: services.StoragePool, upstream: services.UpstreamPool, newClient: liveCnkiClient(options.ProviderProxy.ForProvider(sources.ZjlibProviderName))}).routes(),
		(&adminHandlers{auth: authHandlers, storage: services.Storage, scheduler: services.Scheduler, settings: settingsRepository, telemetry: services.Telemetry}).routes(),
		(&cfpHandlers{services.Storage, services.Cfp, services.Codec, authenticator, services.StoragePool}).routes(),
		(&articleHandlers{storage: services.Storage, authenticator: authenticator, settings: settingsRepository, sessions: sessions, providers: providers, pool: services.StoragePool, upstream: services.UpstreamPool}).routes(),
		(&trackingHandlers{storage: services.Storage, repository: services.Delivery, favorites: favoriteRepository, settings: settingsRepository, codec: services.Codec, authenticator: authenticator, pool: services.StoragePool}).routes(),
	}
	mux := http.NewServeMux()
	contract := openapi.New(mux)
	for _, group := range groups {
		for _, candidate := range group {
			if err := candidate.register(contract); err != nil {
				return nil, err
			}
		}
	}
	server, err := mcp.NewServer(mcp.Services{Storage: services.Storage, Favorites: favoriteRepository, Pool: services.StoragePool, Telemetry: services.Telemetry})
	if err != nil {
		return nil, err
	}
	options.TrustedProxies = slices.Clone(options.TrustedProxies)
	options.CorsOrigins = slices.Clone(options.CorsOrigins)
	options.McpHosts = slices.Clone(options.McpHosts)
	options.McpOrigins = slices.Clone(options.McpOrigins)
	handler := mcpcompat.NewWithPolicy(server, authenticator.McpAuthorize, mcpcompat.HostOriginPolicy{AllowedHosts: options.McpHosts, AllowedOrigins: options.McpOrigins})
	mux.Handle("/mcp", handler)
	mux.Handle("/mcp/", handler)
	mux.HandleFunc("GET /docs/{$}", func(writer http.ResponseWriter, request *http.Request) {
		http.Redirect(writer, request, "/docs", http.StatusSeeOther)
	})
	return &Handler{telemetry: services.Telemetry, mux: mux, mcp: handler, options: options}, nil
}

// validateAPIServices checks borrowed resources before validating router options.
func validateAPIServices(services Services) error {
	if services.Auth == nil || services.Cfp == nil || services.Delivery == nil || services.Scheduler == nil || services.Codec == nil || services.StoragePool == nil || services.UpstreamPool == nil || services.KdfPool == nil {
		return errors.New("API services must be initialized before router construction")
	}
	return nil
}

// Close closes MCP sessions; borrowed repositories and executors remain owned by the host.
func (handler *Handler) Close() error { return handler.mcp.Close() }

// GenerateOpenAPI verifies the real handler declarations without opening deployment resources.
func GenerateOpenAPI() ([]byte, error) {
	contract, err := newDocumentationAPI()
	if err != nil {
		return nil, err
	}
	return json.Marshal(contract.OpenAPI())
}

func newDocumentationAPI() (huma.API, error) {
	groups := [][]route{
		(&authHandlers{}).routes(), (&indexHandlers{}).routes(), (&favoriteHandlers{}).routes(),
		(&favoriteHandlers{}).graphRoutes(),
		(&publicHandlers{}).routes(), (&cnkiHandlers{}).routes(), (&adminHandlers{}).routes(),
		(&cfpHandlers{}).routes(), (&articleHandlers{}).routes(), (&trackingHandlers{}).routes(),
	}
	contract := openapi.New(http.NewServeMux())
	for _, group := range groups {
		for _, candidate := range group {
			if err := candidate.register(contract); err != nil {
				return nil, err
			}
		}
	}
	return contract, nil
}

// route obtains bounded telemetry labels and method policy from ServeMux.
func (handler *Handler) route(request *http.Request) (http.Handler, string, string) {
	selected, pattern := handler.mux.Handler(request)
	if pattern == "" {
		response := &routeResponse{headers: make(http.Header)}
		selected.ServeHTTP(response, request)
		switch response.status {
		case http.StatusNotFound:
			return nil, unmatchedRoute(request.URL.EscapedPath()), ""
		case http.StatusMethodNotAllowed:
			return nil, unmatchedRoute(request.URL.EscapedPath()), response.headers.Get("Allow")
		default:
			return selected, unmatchedRoute(request.URL.EscapedPath()), ""
		}
	}
	if _, path, hasMethod := strings.Cut(pattern, " "); hasMethod {
		pattern = path
	}
	return http.HandlerFunc(handler.mux.ServeHTTP), pattern, ""
}

type routeResponse struct {
	headers http.Header
	status  int
}

// Header retains the standard router's method metadata.
func (response *routeResponse) Header() http.Header { return response.headers }

// WriteHeader records a router rejection without committing the public response.
func (response *routeResponse) WriteHeader(status int) { response.status = status }

// Write discards the standard router's plain-text rejection body.
func (response *routeResponse) Write(data []byte) (int, error) { return len(data), nil }

func unmatchedRoute(path string) string {
	for _, prefix := range []string{"/api", "/mcp", "/health"} {
		if path == prefix || strings.HasPrefix(path, prefix+"/") {
			return prefix[1:] + ".unmatched"
		}
	}
	return "http.unmatched"
}

func withRequestID(request *http.Request, id string) *http.Request {
	request = request.Clone(context.WithValue(request.Context(), requestIDKey{}, id))
	request.Header.Del("X-Request-Id")
	request.Header.Set("X-Request-Id", id)
	return request
}
