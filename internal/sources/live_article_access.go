package sources

import (
	"context"
	"strings"
	"time"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/zjlib"
	authstorage "github.com/QianFuv/LitRadar/internal/storage/auth"
	"github.com/QianFuv/LitRadar/internal/transport"
)

// LiveCnkiArticleAccess creates a private domestic session for each request deadline.
type LiveCnkiArticleAccess struct {
	create func(time.Time) (cnki.Transport, func(), error)
}

// NewLiveCnkiArticleAccess retains explicit proxy and captcha configuration, never session cookies.
func NewLiveCnkiArticleAccess(captchaToken *string, proxy transport.Proxy) *LiveCnkiArticleAccess {
	if captchaToken != nil {
		captchaToken = new(*captchaToken)
	}
	return &LiveCnkiArticleAccess{create: func(deadline time.Time) (cnki.Transport, func(), error) {
		client, err := cnki.NewLiveTransport(cnki.LiveConfig{TimeoutSeconds: 30, CaptchaToken: captchaToken}, proxy, deadline)
		if err != nil {
			return nil, func() {}, err
		}
		return client, client.Close, nil
	}}
}

// SupportsAbstract shares the same bibliographic predicate as retained-session access.
func (*LiveCnkiArticleAccess) SupportsAbstract(article domain.ArticleLocator) bool {
	return (&CnkiArticleAccess{}).SupportsAbstract(article)
}

// ResolveAbstract confines cookies, cleanup and networking to this invocation's shared deadline.
func (access *LiveCnkiArticleAccess) ResolveAbstract(ctx context.Context, article domain.ArticleLocator, request domain.ArticleAccessContext) (domain.ArticleRedirect, error) {
	client, close, err := access.create(request.Deadline)
	if err != nil {
		return domain.ArticleRedirect{}, accessFailure(provider.TemporarilyUnavailable, "domestic CNKI transport is unavailable")
	}
	defer close()
	return NewCnkiArticleAccess(client).ResolveAbstract(ctx, article, request)
}

// LiveCnkiAccessRegistration declares the request-scoped domestic abstract capability.
func LiveCnkiAccessRegistration(captchaToken *string, proxy transport.Proxy) (*provider.Registration, error) {
	return CnkiAccessRegistration(NewLiveCnkiArticleAccess(captchaToken, proxy))
}

type cnkiSessionReader interface {
	Data(context.Context, identity.Id, bool) (*authstorage.CnkiData, error)
}

// ZjlibFullTextProvider resolves private documents using the requesting user's active stored session.
type ZjlibFullTextProvider struct {
	sessions cnkiSessionReader
	create   func(time.Time) (zjlib.Transport, func(), error)
}

// NewZjlibFullTextProvider uses read-only session access and a fresh library transport per resolution.
func NewZjlibFullTextProvider(sessions cnkiSessionReader, proxy transport.Proxy) *ZjlibFullTextProvider {
	return &ZjlibFullTextProvider{sessions: sessions, create: func(deadline time.Time) (zjlib.Transport, func(), error) {
		client, err := zjlib.NewLiveTransport(zjlib.DefaultLiveConfig(), proxy, deadline)
		if err != nil {
			return nil, func() {}, err
		}
		return client, client.Close, nil
	}}
}

// SupportsFullText requires the title, journal and at least one named author used by exact matching.
func (*ZjlibFullTextProvider) SupportsFullText(article domain.ArticleLocator) bool {
	if strings.TrimSpace(article.Title) == "" || strings.TrimSpace(article.JournalTitle) == "" {
		return false
	}
	for _, author := range article.Authors {
		if strings.TrimSpace(author) != "" {
			return true
		}
	}
	return false
}

// ResolveFullText loads active session state, warms it and checks ten ordered candidates without persistence.
func (access *ZjlibFullTextProvider) ResolveFullText(ctx context.Context, article domain.ArticleLocator, request domain.ArticleAccessContext) (domain.ArticleFullTextResolution, error) {
	if request.UserId == nil {
		return domain.ArticleFullTextResolution{}, accessFailure(provider.AuthenticationRequired, "authenticated CNKI session required")
	}
	session, err := access.sessions.Data(ctx, *request.UserId, true)
	if err != nil {
		return domain.ArticleFullTextResolution{}, accessFailure(provider.Internal, "CNKI session unavailable")
	}
	if session == nil {
		return domain.ArticleFullTextResolution{}, accessFailure(provider.AuthenticationRequired, "active CNKI session required")
	}
	state, err := transport.ParseJson(session.SessionData)
	if err != nil {
		return domain.ArticleFullTextResolution{}, accessFailure(provider.Internal, "CNKI session unavailable")
	}
	wire, close, err := access.create(request.Deadline)
	if err != nil {
		return domain.ArticleFullTextResolution{}, mapZjlibProviderError(err)
	}
	defer close()
	client := zjlib.NewClient(wire)
	client.LoadStateData(state)
	if _, err := client.WarmUpFulltextSession(ctx); err != nil {
		return domain.ArticleFullTextResolution{}, mapZjlibProviderError(err)
	}
	expected := zjlib.ArticleIdentity{Title: article.Title, Authors: strings.Join(article.Authors, "; "), JournalTitle: article.JournalTitle}
	downloaded, err := client.DownloadMatchingPdf(ctx, expected, 10)
	if err != nil {
		return domain.ArticleFullTextResolution{}, mapZjlibProviderError(err)
	}
	return domain.ArticleFullTextResolution{Document: &domain.ArticleFullTextDocument{ContentType: asciiLowerSource(downloaded.ContentType), Filename: &downloaded.Filename, Bytes: downloaded.Content}}, nil
}

func mapZjlibProviderError(err error) error {
	kind := provider.TemporarilyUnavailable
	if strings.Contains(err.Error(), "No exact CNKI full-text match") {
		kind = provider.NotFound
	} else if strings.Contains(err.Error(), "Run QR login") || strings.Contains(err.Error(), "token") {
		kind = provider.AuthenticationRequired
	}
	return accessFailure(kind, "Zhejiang Library CNKI full-text resolution failed")
}

// ZjlibFullTextRegistration declares documents only, without redirect authority.
func ZjlibFullTextRegistration(sessions cnkiSessionReader, proxy transport.Proxy) (*provider.Registration, error) {
	return provider.NewRegistration(provider.Descriptor{Name: ZjlibProviderName, Capabilities: provider.Capabilities{ArticleFullText: true}}, provider.Implementations{ArticleFullText: NewZjlibFullTextProvider(sessions, proxy)})
}
