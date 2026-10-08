package api

import (
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"slices"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/openapi"
	"github.com/QianFuv/LitRadar/internal/platform/admission"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources"
	storage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type articleHandlers struct {
	storage        config.Config
	authenticator  *Authenticator
	settings       *settings.Repository
	sessions       *storage.CnkiSessions
	providers      *provider.Registry
	pool, upstream *executor.Pool
}
type articleOrders struct{ abstract, fulltext settings.ProviderOrders }
type articleAction struct {
	Available     bool    `json:"available"`
	Label         string  `json:"label"`
	RequiresLogin bool    `json:"requires_login"`
	Message       *string `json:"message"`
}
type articleAccess struct {
	AbstractPage articleAction `json:"abstract_page"`
	Fulltext     articleAction `json:"fulltext"`
}

func newArticleProviders(repository *settings.Repository, sessions *storage.CnkiSessions, proxy sources.ProxySelection) (*provider.Registry, error) {
	var captcha *string
	if values, err := repository.Load(context.Background()); err == nil {
		for _, value := range values {
			if value.Field == "cnki_captcha_token" && strings.TrimSpace(value.Value) != "" {
				captcha = &value.Value
			}
		}
	}
	registry := &provider.Registry{}
	for _, build := range []func() (*provider.Registration, error){sources.ScholarlyAccessRegistration, func() (*provider.Registration, error) {
		return sources.LiveCnkiAccessRegistration(captcha, proxy.ForProvider(sources.CnkiProviderName))
	}, func() (*provider.Registration, error) {
		return sources.ZjlibFullTextRegistration(sessions, proxy.ForProvider(sources.ZjlibProviderName))
	}} {
		registration, err := build()
		if err != nil {
			return nil, err
		}
		if err := registry.Register(registration); err != nil {
			return nil, err
		}
	}
	return registry, nil
}

func (handlers *articleHandlers) routes() []route {
	routes := []route{}
	for _, operation := range []openapi.Operation{{Method: "GET", Path: "/api/articles/{article_id}/access", Id: "get_article_access"}, {Method: "GET", Path: "/api/articles/{article_id}/abstract", Id: "redirect_article_abstract"}, {Method: "GET", Path: "/api/articles/{article_id}/fulltext", Id: "redirect_article_fulltext"}} {
		routes = append(routes, route{operation, func(writer http.ResponseWriter, request *http.Request) {
			handlers.handle(writer, request, operation.Id)
		}})
	}
	return routes
}

func (handlers *articleHandlers) handle(writer http.ResponseWriter, request *http.Request, operation string) {
	articleId, failure := extractPathInteger(request, "article_id")
	if failure != nil {
		failure.write(writer)
		return
	}
	query, failure := extractQuery(request.URL.RawQuery, map[string]queryKind{"db": queryText})
	if failure != nil {
		failure.write(writer)
		return
	}
	current, failure := handlers.authenticator.requireUser(request)
	if failure != nil {
		failure.write(writer)
		return
	}
	type locatorResult struct {
		article domain.ArticleLocator
		stem    string
		err     error
	}
	located, err := executor.Run(request.Context(), handlers.pool, func() (locatorResult, error) {
		stem, err := handlers.storage.ResolveIndexCatalogStem(query.database())
		if err != nil {
			return locatorResult{err: err}, nil
		}
		article, err := sources.GetArticleLocator(context.WithoutCancel(request.Context()), handlers.storage, query.database(), identity.Id(articleId))
		return locatorResult{article, stem, err}, nil
	})
	if err != nil {
		mapExecutorError(err).write(writer)
		return
	}
	if located.err != nil {
		mapIndexError(located.err).write(writer)
		return
	}
	user := current.authorization.User.Id
	if operation == "get_article_access" {
		status, failure := handlers.status(request.Context(), located.article, user, located.stem)
		if failure != nil {
			failure.write(writer)
		} else {
			writeResponse(writer, status)
		}
		return
	}
	action := "abstract"
	if operation == "redirect_article_fulltext" {
		action = "fulltext"
	}
	resolved, failure := handlers.resolve(request.Context(), located.article, user, located.stem, action, time.Now().Add(30*time.Second))
	if failure != nil {
		failure.write(writer)
		return
	}
	writeArticleResolution(writer, resolved)
}

func (handlers *articleHandlers) loadOrders(ctx context.Context) (articleOrders, *apiError) {
	type result struct {
		values []settings.Value
		err    error
	}
	value, err := executor.Run(ctx, handlers.pool, func() (result, error) {
		values, err := handlers.settings.Load(context.WithoutCancel(ctx))
		return result{values, err}, nil
	})
	if err != nil {
		return articleOrders{}, mapExecutorError(err)
	}
	if value.err != nil {
		return articleOrders{}, internalError()
	}
	orders := articleOrders{}
	for _, setting := range value.values {
		var target *settings.ProviderOrders
		switch setting.Field {
		case "article_abstract_provider_orders":
			target = &orders.abstract
		case "article_fulltext_provider_orders":
			target = &orders.fulltext
		}
		if target != nil && json.Unmarshal([]byte(setting.Value), target) != nil {
			return articleOrders{}, internalError()
		}
	}
	return orders, nil
}
func articleOrder(configuration settings.ProviderOrders, catalog string) []string {
	if order, exists := configuration.Catalogs[catalog]; exists {
		return order
	}
	return configuration.Default
}
func articleSupports(registration *provider.Registration, article domain.ArticleLocator, action string) bool {
	if registration == nil {
		return false
	}
	if action == "abstract" {
		return registration.ArticleAbstract() != nil && registration.ArticleAbstract().SupportsAbstract(article)
	}
	return registration.ArticleFullText() != nil && registration.ArticleFullText().SupportsFullText(article)
}

func (handlers *articleHandlers) status(ctx context.Context, article domain.ArticleLocator, user identity.Id, catalog string) (*articleAccess, *apiError) {
	orders, failure := handlers.loadOrders(ctx)
	if failure != nil {
		return nil, failure
	}
	type sessionResult struct {
		hasSession bool
		err        error
	}
	session, err := executor.Run(ctx, handlers.pool, func() (sessionResult, error) {
		value, err := handlers.sessions.Data(context.WithoutCancel(ctx), user, true)
		return sessionResult{value != nil, err}, nil
	})
	if err != nil {
		return nil, mapExecutorError(err)
	}
	if session.err != nil {
		return nil, internalError()
	}
	fulltext := articleOrder(orders.fulltext, catalog)
	hasWithoutLogin, hasWithLogin := false, false
	for _, name := range fulltext {
		if articleSupports(handlers.providers.Find(name), article, "fulltext") {
			if name == sources.ZjlibProviderName {
				hasWithLogin = true
			} else {
				hasWithoutLogin = true
			}
		}
	}
	return &articleAccess{handlers.actionStatus(articleOrder(orders.abstract, catalog), article, "abstract", "查看摘要页", false), handlers.actionStatus(fulltext, article, "fulltext", "获取全文", !session.hasSession && !hasWithoutLogin && hasWithLogin)}, nil
}

// actionStatus distinguishes missing capabilities, article data and login requirements.
func (handlers *articleHandlers) actionStatus(order []string, article domain.ArticleLocator, action, label string, requiresLogin bool) articleAction {
	hasConfigured, hasProvider := false, false
	for _, name := range order {
		registration := handlers.providers.Find(name)
		if registration == nil {
			continue
		}
		if hasArticleCapability(registration, action) {
			hasConfigured = true
		}
		hasProvider = hasProvider || articleSupports(registration, article, action)
	}
	var message *string
	value := ""
	switch {
	case !hasConfigured:
		value = "当前未配置可用的在线能力"
	case !hasProvider:
		value = "当前文章缺少可用于在线解析的信息"
	case requiresLogin:
		value = "请先完成浙江图书馆 CNKI 登录"
	}
	if value != "" {
		message = &value
	}
	return articleAction{hasProvider && !requiresLogin, label, requiresLogin, message}
}

// hasArticleCapability checks only the requested registered action.
func hasArticleCapability(registration *provider.Registration, action string) bool {
	return action == "abstract" && registration.ArticleAbstract() != nil || action == "fulltext" && registration.ArticleFullText() != nil
}

type articleFailures struct{ hasAuthentication, hasRetryable, hasGateway bool }

func (failures *articleFailures) record(ctx context.Context, name, action string, kind provider.ErrorKind, reason string) {
	switch kind {
	case provider.AuthenticationRequired:
		failures.hasAuthentication = true
	case provider.TemporarilyUnavailable:
		failures.hasRetryable = true
	case provider.InvalidResponse, provider.Internal:
		failures.hasGateway = true
	}
	slog.DebugContext(ctx, "article.access.fallback", "event", "article.access.fallback", "component", "article_access", "provider", name, "action", action, "reason", reason)
}
func (failures *articleFailures) providerError(ctx context.Context, name, action string, err error) {
	var failure *provider.Error
	kind := provider.Internal
	if errors.As(err, &failure) {
		kind = failure.Kind
	}
	reason := map[provider.ErrorKind]string{provider.NotFound: "not_found", provider.AuthenticationRequired: "authentication_required", provider.TemporarilyUnavailable: "temporarily_unavailable", provider.InvalidResponse: "invalid_response", provider.Internal: "internal"}[kind]
	failures.record(ctx, name, action, kind, reason)
}
func (failures articleFailures) response(action string) *apiError {
	if failures.hasAuthentication {
		return &apiError{status: 428, structured: map[string]string{"code": "article_access_authentication_required", "action": action, "message": "Complete the configured provider login before retrying this action."}}
	}
	if failures.hasRetryable {
		retry := uint64(5)
		return &apiError{status: 503, detail: "Article provider temporarily unavailable", retryAfter: &retry}
	}
	if failures.hasGateway {
		return &apiError{status: 502, detail: "Article provider request failed"}
	}
	detail := "Article abstract action is unavailable"
	if action == "fulltext" {
		detail = "Article full text is unavailable"
	}
	return &apiError{status: 404, detail: detail}
}

// resolve preserves provider order, the shared deadline and accumulated failure priority.
func (handlers *articleHandlers) resolve(ctx context.Context, article domain.ArticleLocator, user identity.Id, catalog, action string, deadline time.Time) (domain.ArticleFullTextResolution, *apiError) {
	empty := domain.ArticleFullTextResolution{}
	if provider.ValidateArticleLocator(article) != nil {
		return empty, internalError()
	}
	orders, failure := handlers.loadOrders(ctx)
	if failure != nil {
		return empty, failure
	}
	order := articleOrder(orders.abstract, catalog)
	if action == "fulltext" {
		order = articleOrder(orders.fulltext, catalog)
	}
	request := domain.ArticleAccessContext{UserId: &user, Deadline: deadline}
	failures := articleFailures{}
	for _, name := range order {
		registration := handlers.providers.Find(name)
		if !articleSupports(registration, article, action) {
			continue
		}
		resolution, isResolved, shouldStop := handlers.resolveProvider(ctx, name, registration, article, request, action, &failures)
		if isResolved {
			return resolution, nil
		}
		if shouldStop {
			break
		}
	}
	return empty, failures.response(action)
}

type articleProviderResult struct {
	resolution domain.ArticleFullTextResolution
	err        error
}

// resolveProvider admits one provider and discards results that arrive after the shared deadline.
// An unresolved result either permits fallback or stops it; a resolved result never requests a stop.
func (handlers *articleHandlers) resolveProvider(ctx context.Context, name string, registration *provider.Registration, article domain.ArticleLocator, request domain.ArticleAccessContext, action string, failures *articleFailures) (resolution domain.ArticleFullTextResolution, isResolved, shouldStop bool) {
	empty := domain.ArticleFullTextResolution{}
	remaining := time.Until(request.Deadline)
	if remaining <= 0 {
		failures.record(ctx, name, action, provider.TemporarilyUnavailable, "deadline_expired")
		return empty, false, true
	}
	value, err := executor.RunWithQueueTimeout(ctx, handlers.upstream, min(30*time.Second, remaining), func() (articleProviderResult, error) {
		if action == "abstract" {
			redirect, err := registration.ArticleAbstract().ResolveAbstract(context.WithoutCancel(ctx), article, request)
			return articleProviderResult{domain.ArticleFullTextResolution{Redirect: &redirect}, err}, nil
		}
		resolution, err := registration.ArticleFullText().ResolveFullText(context.WithoutCancel(ctx), article, request)
		return articleProviderResult{resolution, err}, nil
	})
	if err != nil {
		return empty, false, failures.executionError(ctx, name, action, request.Deadline, err)
	}
	if time.Until(request.Deadline) <= 0 {
		if value.err != nil {
			failures.providerError(ctx, name, action, value.err)
		}
		failures.record(ctx, name, action, provider.TemporarilyUnavailable, "deadline_expired")
		return empty, false, true
	}
	if value.err != nil {
		failures.providerError(ctx, name, action, value.err)
		return empty, false, false
	}
	if isApprovedArticleResolution(registration, value.resolution) {
		return value.resolution, true, false
	}
	failures.record(ctx, name, action, provider.InvalidResponse, "invalid_response")
	return empty, false, false
}

// executionError records admission or join failures and decides whether fallback can continue.
func (failures *articleFailures) executionError(ctx context.Context, name, action string, deadline time.Time, err error) bool {
	switch {
	case errors.Is(err, admission.ErrClosed):
		failures.record(ctx, name, action, provider.TemporarilyUnavailable, "executor_closed")
	case errors.Is(err, context.DeadlineExceeded):
		failures.record(ctx, name, action, provider.TemporarilyUnavailable, "queue_timeout")
	default:
		failures.record(ctx, name, action, provider.Internal, "executor_join_failed")
	}
	return errors.Is(err, admission.ErrClosed) || time.Until(deadline) <= 0 || ctx.Err() != nil
}

// isApprovedArticleResolution enforces provider redirect hosts and PDF document responses.
func isApprovedArticleResolution(registration *provider.Registration, resolution domain.ArticleFullTextResolution) bool {
	isValid := provider.ValidateFullTextResolution(resolution, 32*1024*1024) == nil
	if resolution.Redirect != nil {
		return isValid && approvedArticleRedirect(registration.Descriptor().AllowedRedirectHosts, resolution.Redirect.Location)
	}
	if resolution.Document != nil {
		return isValid && resolution.Document.ContentType == "application/pdf"
	}
	return isValid
}

func approvedArticleRedirect(allowed []string, location string) bool {
	remainder, ok := strings.CutPrefix(location, "https://")
	if !ok {
		return false
	}
	authority := remainder
	if index := strings.IndexAny(authority, "/?#"); index >= 0 {
		authority = authority[:index]
	}
	if index := strings.LastIndexByte(authority, ':'); index >= 0 {
		port := authority[index+1:]
		if !strings.ContainsFunc(port, func(character rune) bool { return character < '0' || character > '9' }) {
			authority = authority[:index]
		}
	}
	return slices.Contains(allowed, asciiLower(authority))
}

func writeArticleResolution(writer http.ResponseWriter, resolution domain.ArticleFullTextResolution) {
	writer.Header().Set("Cache-Control", "private, no-store")
	if resolution.Redirect != nil {
		writer.Header().Set("Location", resolution.Redirect.Location)
		writer.WriteHeader(http.StatusTemporaryRedirect)
		return
	}
	document := resolution.Document
	writer.Header().Set("Content-Type", document.ContentType)
	if document.Filename != nil {
		writer.Header().Set("Content-Disposition", "attachment; filename*=UTF-8''"+articleFilename(*document.Filename))
	}
	writer.WriteHeader(200)
	_, _ = writer.Write(document.Bytes)
}
func articleFilename(filename string) string {
	var result strings.Builder
	const hex = "0123456789ABCDEF"
	for _, value := range []byte(filename) {
		if value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9' || strings.ContainsRune("-_.~", rune(value)) {
			result.WriteByte(value)
		} else {
			result.WriteByte('%')
			result.WriteByte(hex[value>>4])
			result.WriteByte(hex[value&15])
		}
	}
	return result.String()
}
