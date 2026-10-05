package api

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type articleFixtureProvider struct {
	hasSupport bool
	resolve    func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error)
}

func (fixture articleFixtureProvider) SupportsAbstract(domain.ArticleLocator) bool {
	return fixture.hasSupport
}
func (fixture articleFixtureProvider) SupportsFullText(domain.ArticleLocator) bool {
	return fixture.hasSupport
}
func (fixture articleFixtureProvider) ResolveAbstract(_ context.Context, _ domain.ArticleLocator, request domain.ArticleAccessContext) (domain.ArticleRedirect, error) {
	value, err := fixture.resolve(request)
	if value.Redirect != nil {
		return *value.Redirect, err
	}
	return domain.ArticleRedirect{}, err
}
func (fixture articleFixtureProvider) ResolveFullText(_ context.Context, _ domain.ArticleLocator, request domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
	return fixture.resolve(request)
}

func articleFixture(t *testing.T) (*articleHandlers, *authHandlers, *http.ServeMux, string) {
	t.Helper()
	auth, router, token := authFixture(t)
	codec, err := secrets.NewCodec(bytes.Repeat([]byte{42}, 32))
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(codec.Close)
	upstream := executor.New(2, 30*time.Second)
	t.Cleanup(upstream.Close)
	handlers := &articleHandlers{storage: config.FromProjectRoot(t.TempDir()), authenticator: auth.authenticator, settings: settings.New(auth.repository, codec), sessions: storage.NewCnkiSessions(auth.repository, codec), providers: &provider.Registry{}, pool: auth.pool, upstream: upstream}
	for _, route := range handlers.routes() {
		router.HandleFunc(route.operation.Method+" "+route.operation.Path, route.handler)
	}
	return handlers, auth, router, token
}
func articleLocatorFixture() domain.ArticleLocator {
	return domain.ArticleLocator{ArticleId: 1, CatalogId: "fixture-journal", JournalTitle: "Fixture Journal", Title: "Fixture article", Authors: []string{"Author"}, JournalIssns: []string{}}
}
func registerArticleFixture(t *testing.T, handlers *articleHandlers, name string, fixture articleFixtureProvider) {
	t.Helper()
	registration, err := provider.NewRegistration(provider.Descriptor{Name: name, Capabilities: provider.Capabilities{ArticleAbstract: true, ArticleFullText: true}, AllowedRedirectHosts: []string{"example.org"}}, provider.Implementations{ArticleAbstract: fixture, ArticleFullText: fixture})
	if err != nil {
		t.Fatal(err)
	}
	if err := handlers.providers.Register(registration); err != nil {
		t.Fatal(err)
	}
}
func setArticleOrder(t *testing.T, handlers *articleHandlers, value string) {
	t.Helper()
	if _, err := handlers.settings.Update(context.Background(), nil, map[string]*string{"article_abstract_provider_orders": &value, "article_fulltext_provider_orders": &value}, nil, nil); err != nil {
		t.Fatal(err)
	}
}

func TestArticleFallbackErrorPriorityAndSafeResponse(t *testing.T) {
	for _, scenario := range []struct {
		name   string
		kinds  []provider.ErrorKind
		status int
	}{
		{"miss", []provider.ErrorKind{provider.NotFound}, 404},
		{"gateway", []provider.ErrorKind{provider.InvalidResponse, provider.NotFound}, 502},
		{"retry", []provider.ErrorKind{provider.Internal, provider.TemporarilyUnavailable}, 503},
		{"login", []provider.ErrorKind{provider.TemporarilyUnavailable, provider.AuthenticationRequired, provider.InvalidResponse}, 428},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			handlers, _, _, _ := articleFixture(t)
			names := []string{}
			for index, kind := range scenario.kinds {
				name := "fixture-" + string(rune('a'+index))
				names = append(names, name)
				registerArticleFixture(t, handlers, name, articleFixtureProvider{true, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
					return domain.ArticleFullTextResolution{}, &provider.Error{Kind: kind, Message: "private provider secret"}
				}})
			}
			encoded, _ := json.Marshal(settings.ProviderOrders{Default: names, Catalogs: map[string][]string{}})
			setArticleOrder(t, handlers, string(encoded))
			for _, action := range []string{"abstract", "fulltext"} {
				_, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "fixture", action, time.Now().Add(time.Second))
				if failure == nil || failure.status != scenario.status {
					t.Fatal(action, failure)
				}
				writer := httptest.NewRecorder()
				failure.write(writer)
				if strings.Contains(writer.Body.String(), "private") {
					t.Fatal(writer.Body.String())
				}
				if scenario.status == 503 && writer.Header().Get("Retry-After") != "5" {
					t.Fatal(writer.Header())
				}
				if scenario.status == 428 && !strings.Contains(writer.Body.String(), `"action":"`+action+`"`) {
					t.Fatal(writer.Body.String())
				}
			}
		})
	}
}

func TestArticleSharedDeadlineDiscardsLateResultsAndPreservesLoginPriority(t *testing.T) {
	for _, kind := range []provider.ErrorKind{"", provider.AuthenticationRequired} {
		handlers, _, _, _ := articleFixture(t)
		var laterCalls atomic.Int64
		var observed time.Time
		registerArticleFixture(t, handlers, "late", articleFixtureProvider{true, func(request domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
			observed = request.Deadline
			time.Sleep(max(0, time.Until(request.Deadline)) + 5*time.Millisecond)
			if kind != "" {
				return domain.ArticleFullTextResolution{}, &provider.Error{Kind: kind, Message: "private"}
			}
			return domain.ArticleFullTextResolution{Redirect: &domain.ArticleRedirect{Location: "https://example.org/paper"}}, nil
		}})
		registerArticleFixture(t, handlers, "later", articleFixtureProvider{true, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
			laterCalls.Add(1)
			return domain.ArticleFullTextResolution{}, nil
		}})
		setArticleOrder(t, handlers, `{"default":["late","later"],"catalogs":{}}`)
		deadline := time.Now().Add(100 * time.Millisecond)
		_, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "fixture", "abstract", deadline)
		status := 503
		if kind == provider.AuthenticationRequired {
			status = 428
		}
		if failure == nil || failure.status != status || !observed.Equal(deadline) || laterCalls.Load() != 0 {
			t.Fatal(failure, observed, deadline, laterCalls.Load())
		}
	}
}

func TestArticleFallbackSkipsUnsupportedAndSharesDeadline(t *testing.T) {
	handlers, _, _, _ := articleFixture(t)
	var observed []time.Time
	registerArticleFixture(t, handlers, "unsupported", articleFixtureProvider{false, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
		panic("unsupported provider invoked")
	}})
	registerArticleFixture(t, handlers, "miss", articleFixtureProvider{true, func(request domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
		observed = append(observed, request.Deadline)
		return domain.ArticleFullTextResolution{}, &provider.Error{Kind: provider.NotFound}
	}})
	registerArticleFixture(t, handlers, "success", articleFixtureProvider{true, func(request domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
		observed = append(observed, request.Deadline)
		return domain.ArticleFullTextResolution{Redirect: &domain.ArticleRedirect{Location: "https://EXAMPLE.ORG:443/article?q=1"}}, nil
	}})
	setArticleOrder(t, handlers, `{"default":["missing","unsupported","miss","success"],"catalogs":{"disabled":[]}}`)
	deadline := time.Now().Add(time.Second)
	value, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "fixture", "abstract", deadline)
	if failure != nil || value.Redirect == nil || len(observed) != 2 || !observed[0].Equal(deadline) || !observed[1].Equal(deadline) {
		t.Fatal(value, failure, observed)
	}
	if _, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "disabled", "abstract", deadline); failure == nil || failure.status != 404 {
		t.Fatal(failure)
	}
	handlers.upstream.Close()
	if _, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "fixture", "abstract", deadline); failure == nil || failure.status != 503 {
		t.Fatal(failure)
	}
}

func TestArticleResolutionRejectsUnsafeRedirectsAndNonPdf(t *testing.T) {
	for _, location := range []string{"http://example.org/a", "https://example.org.evil/a", "https://user@example.org/a", "https://example.org:bad/a", "https://example.org/a\r\nX: 1"} {
		handlers, _, _, _ := articleFixture(t)
		registerArticleFixture(t, handlers, "fixture", articleFixtureProvider{true, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
			return domain.ArticleFullTextResolution{Redirect: &domain.ArticleRedirect{Location: location}}, nil
		}})
		setArticleOrder(t, handlers, `{"default":["fixture"],"catalogs":{}}`)
		if _, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "fixture", "abstract", time.Now().Add(time.Second)); failure == nil || failure.status != 502 {
			t.Fatal(location, failure)
		}
	}
	for _, contentType := range []string{"application/pdf", "text/html"} {
		handlers, _, _, _ := articleFixture(t)
		registerArticleFixture(t, handlers, "fixture", articleFixtureProvider{true, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
			return domain.ArticleFullTextResolution{Document: &domain.ArticleFullTextDocument{ContentType: contentType, Bytes: []byte("document")}}, nil
		}})
		setArticleOrder(t, handlers, `{"default":["fixture"],"catalogs":{}}`)
		_, failure := handlers.resolve(context.Background(), articleLocatorFixture(), 1, "fixture", "fulltext", time.Now().Add(time.Second))
		if contentType == "application/pdf" {
			if failure != nil {
				t.Fatal(failure)
			}
		} else if failure == nil || failure.status != 502 {
			t.Fatal(failure)
		}
	}
}

func TestArticleStatusUsesCurrentSessionAndLoadsAllSettings(t *testing.T) {
	handlers, auth, _, _ := articleFixture(t)
	registerArticleFixture(t, handlers, "zjlib", articleFixtureProvider{true, func(domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
		panic("status must not resolve providers")
	}})
	setArticleOrder(t, handlers, `{"default":["zjlib"],"catalogs":{"disabled":[]}}`)
	status, failure := handlers.status(context.Background(), articleLocatorFixture(), 1, "fixture")
	if failure != nil || status.Fulltext.Available || !status.Fulltext.RequiresLogin {
		t.Fatal(status, failure)
	}
	if _, err := handlers.sessions.Upsert(context.Background(), 1, json.RawMessage(`{"token":"fixture","expires_in":3600}`), "active", nil); err != nil {
		t.Fatal(err)
	}
	status, failure = handlers.status(context.Background(), articleLocatorFixture(), 1, "fixture")
	if failure != nil || !status.Fulltext.Available || status.Fulltext.RequiresLogin {
		t.Fatal(status, failure)
	}
	status, failure = handlers.status(context.Background(), articleLocatorFixture(), 1, "disabled")
	if failure != nil || status.Fulltext.Available || status.Fulltext.Message == nil || *status.Fulltext.Message != "当前未配置可用的在线能力" {
		t.Fatal(status, failure)
	}
	if err := auth.repository.WithConnection(context.Background(), func(connection *sql.Conn) error {
		_, err := connection.ExecContext(context.Background(), `INSERT INTO runtime_settings(key,value,updated_at) VALUES('openalex_api_key_pool','invalid secret',1)`)
		return err
	}); err != nil {
		t.Fatal(err)
	}
	if _, failure := handlers.status(context.Background(), articleLocatorFixture(), 1, "disabled"); failure == nil || failure.status != 500 {
		t.Fatal("unrelated corrupt setting was ignored", failure)
	}
	registry, err := newArticleProviders(handlers.settings, handlers.sessions, sources.ProxySelection{})
	if err != nil || registry.Find("cnki") == nil {
		t.Fatal("startup captcha load should tolerate unavailable settings", err)
	}
}

func TestArticleHttpExtractionAndResponseHeaders(t *testing.T) {
	_, _, router, token := articleFixture(t)
	for _, scenario := range []struct {
		path, bearer string
		status       int
	}{{"/api/articles/bad/access", "", 400}, {"/api/articles/1/access?db=a&db=b", "", 400}, {"/api/articles/0/access", "", 401}, {"/api/articles/-1/access", token, 404}} {
		response := authRequest(router, "GET", scenario.path, "", scenario.bearer)
		if response.Code != scenario.status {
			t.Fatal(scenario.path, response.Code, response.Body.String())
		}
	}
	writer := httptest.NewRecorder()
	location := "https://example.org/论文?a=1"
	writeArticleResolution(writer, domain.ArticleFullTextResolution{Redirect: &domain.ArticleRedirect{Location: location}})
	if writer.Code != 307 || writer.Body.Len() != 0 || writer.Header().Get("Location") != location || writer.Header().Get("Cache-Control") != "private, no-store" {
		t.Fatal(writer)
	}
	writer = httptest.NewRecorder()
	filename := "论文 file+.pdf"
	writeArticleResolution(writer, domain.ArticleFullTextResolution{Document: &domain.ArticleFullTextDocument{ContentType: "application/pdf", Filename: &filename, Bytes: []byte("%PDF-test")}})
	if writer.Code != 200 || writer.Body.String() != "%PDF-test" || writer.Header().Get("Content-Type") != "application/pdf" || writer.Header().Get("Content-Disposition") != "attachment; filename*=UTF-8''%E8%AE%BA%E6%96%87%20file%2B.pdf" {
		t.Fatal(writer.Header(), writer.Body.String())
	}
}
