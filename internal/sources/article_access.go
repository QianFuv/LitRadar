package sources

import (
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"math"
	"reflect"
	"strconv"
	"strings"
	"sync"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

// ScholarlyArticleAccess derives request-time DOI or PubMed destinations from local identifiers.
type ScholarlyArticleAccess struct{}

// SupportsAbstract reports identifier presence without normalizing or validating caller metadata.
func (ScholarlyArticleAccess) SupportsAbstract(article domain.ArticleLocator) bool {
	return article.Doi != nil && strings.TrimSpace(*article.Doi) != "" || article.Pmid != nil && strings.TrimSpace(*article.Pmid) != ""
}

// ResolveAbstract preserves DOI precedence and encodes its UTF-8 path bytes exactly once.
func (ScholarlyArticleAccess) ResolveAbstract(_ context.Context, article domain.ArticleLocator, _ domain.ArticleAccessContext) (domain.ArticleRedirect, error) {
	if article.Doi != nil {
		return domain.ArticleRedirect{Location: "https://doi.org/" + encodeDoiPath(*article.Doi)}, nil
	}
	if article.Pmid != nil {
		return domain.ArticleRedirect{Location: "https://pubmed.ncbi.nlm.nih.gov/" + *article.Pmid + "/"}, nil
	}
	return domain.ArticleRedirect{}, &provider.Error{Kind: provider.NotFound, Message: "scholarly provider requires a DOI or PubMed identifier"}
}

// ScholarlyAccessRegistration declares only the built-in abstract destination capability.
func ScholarlyAccessRegistration() (*provider.Registration, error) {
	return provider.NewRegistration(provider.Descriptor{Name: ScholarlyProviderName, Capabilities: provider.Capabilities{ArticleAbstract: true}, AllowedRedirectHosts: []string{"doi.org", "pubmed.ncbi.nlm.nih.gov"}}, provider.Implementations{ArticleAbstract: ScholarlyArticleAccess{}})
}

// CnkiArticleAccess serializes bibliographic lookup within one retained domestic session.
type CnkiArticleAccess struct {
	mutex     sync.Mutex
	transport cnki.Transport
}

// NewCnkiArticleAccess retains the supplied transport's session and explicit proxy policy.
func NewCnkiArticleAccess(transport cnki.Transport) *CnkiArticleAccess {
	return &CnkiArticleAccess{transport: transport}
}

// SupportsAbstract requires a searchable title and at least one journal identity field.
func (*CnkiArticleAccess) SupportsAbstract(article domain.ArticleLocator) bool {
	if domain.NormalizeBibliographicText(article.Title) == "" {
		return false
	}
	if domain.NormalizeBibliographicText(article.JournalTitle) != "" {
		return true
	}
	for _, issn := range article.JournalIssns {
		if strings.TrimSpace(issn) != "" {
			return true
		}
	}
	return false
}

// ResolveAbstract finds an exact title and compatible DOI before returning an allowed live destination.
func (access *CnkiArticleAccess) ResolveAbstract(ctx context.Context, article domain.ArticleLocator, _ domain.ArticleAccessContext) (domain.ArticleRedirect, error) {
	access.mutex.Lock()
	defer access.mutex.Unlock()
	defer func() { emitSourceAttemptSummary(ctx, CnkiProviderName, access.transport.DrainAttempts()) }()
	journal, err := access.transport.ResolveJournal(ctx, cnki.NewJournalLocator([]string{article.JournalTitle}, article.JournalIssns))
	if err != nil {
		return domain.ArticleRedirect{}, mapCnkiProviderError(err)
	}
	if journal == nil {
		return domain.ArticleRedirect{}, accessFailure(provider.NotFound, "domestic CNKI provider could not resolve the journal")
	}
	issues, err := access.transport.YearIssues(ctx, journal)
	if err != nil {
		return domain.ArticleRedirect{}, mapCnkiProviderError(err)
	}
	for _, issue := range issues {
		if !cnkiIssueMatchesLocator(issue, article) {
			continue
		}
		redirect, found, err := access.resolveCnkiIssue(ctx, journal, issue, article)
		if found || err != nil {
			return redirect, err
		}
	}
	return domain.ArticleRedirect{}, accessFailure(provider.NotFound, "domestic CNKI provider could not find an exact article match")
}

// CnkiAccessRegistration registers the supplied abstract provider without adding full-text capability.
func CnkiAccessRegistration(implementation provider.ArticleAbstract) (*provider.Registration, error) {
	return provider.NewRegistration(provider.Descriptor{Name: CnkiProviderName, Capabilities: provider.Capabilities{ArticleAbstract: true}, AllowedRedirectHosts: []string{"navi.cnki.net", "kns.cnki.net", "www.cnki.net"}}, provider.Implementations{ArticleAbstract: implementation})
}

func accessFailure(kind provider.ErrorKind, message string) error {
	return &provider.Error{Kind: kind, Message: message}
}

func providerField(value any, name string) any {
	object, _ := value.(map[string]any)
	return object[name]
}

func providerText(value any) *string {
	if text, ok := value.(string); ok {
		return domain.NormalizeText(text)
	}
	var number domain.Number
	var err error
	switch value := value.(type) {
	case domain.Number:
		number = value
	case json.Number:
		number, err = domain.ParseNumber(value)
	case int:
		number, err = domain.ParseNumber(json.Number(strconv.Itoa(value)))
	case int64:
		number, err = domain.ParseNumber(json.Number(strconv.FormatInt(value, 10)))
	case uint64:
		number, err = domain.ParseNumber(json.Number(strconv.FormatUint(value, 10)))
	default:
		return nil
	}
	if err != nil {
		return nil
	}
	text := number.String()
	return &text
}

func cnkiIssueMatchesLocator(issue any, article domain.ArticleLocator) bool {
	var year *int64
	if text := providerText(providerField(issue, "year")); text != nil {
		if parsed, err := strconv.ParseInt(*text, 10, 64); err == nil {
			year = &parsed
		}
	}
	if article.PublicationYear != nil && !reflect.DeepEqual(year, article.PublicationYear) {
		return false
	}
	number := providerText(providerField(issue, "number"))
	return article.IssueNumber == nil || number == nil || domain.NormalizeBibliographicLabel(*article.IssueNumber) == domain.NormalizeBibliographicLabel(*number)
}

func cnkiDetailMatchesLocator(detail any, article domain.ArticleLocator) bool {
	title := providerText(providerField(detail, "title"))
	if title == nil || domain.NormalizeBibliographicText(*title) != domain.NormalizeBibliographicText(article.Title) {
		return false
	}
	var doi *string
	if text := providerText(providerField(detail, "doi")); text != nil {
		doi = domain.NormalizeDoi(*text)
	}
	return article.Doi == nil || doi == nil || *article.Doi == *doi
}

func isPermanentCnkiArticleError(err error) bool {
	var failure *cnki.Error
	if !errors.As(err, &failure) {
		return false
	}
	status, hasStatus := failure.HttpStatus()
	return failure.Kind == "PermanentArticleMissing" || hasStatus && (status == 404 || status == 410)
}

func mapCnkiProviderError(err error) error {
	kind := provider.TemporarilyUnavailable
	var failure *cnki.Error
	if errors.As(err, &failure) {
		switch failure.Kind {
		case "Parse", "MissingFixture":
			kind = provider.InvalidResponse
		case "PermanentArticleMissing":
			kind = provider.NotFound
		}
	}
	return accessFailure(kind, "domestic CNKI provider request failed")
}

func asciiLowerSource(value string) string {
	return strings.Map(func(character rune) rune {
		if character >= 'A' && character <= 'Z' {
			return character + 32
		}
		return character
	}, value)
}

func emitSourceAttemptSummary(ctx context.Context, name string, attempts []scholarly.Attempt) {
	failures, retries := 0, 0
	for _, attempt := range attempts {
		if !attempt.DidSucceed {
			failures++
		}
		if attempt.DidRetry {
			retries++
		}
	}
	slog.InfoContext(ctx, "index.provider.attempts", "event", "index.provider.attempts", "component", "index", "provider", name, "attempt_count", len(attempts), "failure_count", failures, "retry_count", retries)
}

// encodeDoiPath escapes UTF-8 bytes exactly once while retaining the DOI path safe set.
func encodeDoiPath(doi string) string {
	var encoded strings.Builder
	const hex = "0123456789ABCDEF"
	for _, value := range []byte(doi) {
		if isUnescapedDoiByte(value) {
			encoded.WriteByte(value)
		} else {
			encoded.WriteByte('%')
			encoded.WriteByte(hex[value>>4])
			encoded.WriteByte(hex[value&15])
		}
	}
	return encoded.String()
}

// isUnescapedDoiByte admits only the original ASCII alphanumeric and path punctuation set.
func isUnescapedDoiByte(value byte) bool {
	return value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9' || strings.ContainsRune("-._~/", rune(value))
}

// resolveCnkiIssue scans pages in source order and checks overflow only after continuing page work.
func (access *CnkiArticleAccess) resolveCnkiIssue(ctx context.Context, journal, issue any, article domain.ArticleLocator) (domain.ArticleRedirect, bool, error) {
	for pageIndex := uint64(0); ; pageIndex++ {
		page, err := access.transport.IssueArticles(ctx, journal, issue, pageIndex)
		if err != nil {
			return domain.ArticleRedirect{}, false, mapCnkiProviderError(err)
		}
		if page.PageIndex != pageIndex || page.ArticleCount != uint64(len(page.Articles)) {
			return domain.ArticleRedirect{}, false, accessFailure(provider.InvalidResponse, "domestic CNKI issue article page metadata is inconsistent")
		}
		redirect, found, err := access.resolveCnkiPage(ctx, page, article)
		if found || err != nil {
			return redirect, found, err
		}

		if !page.HasNextPage {
			break
		}
		if pageIndex == math.MaxUint64 {
			return domain.ArticleRedirect{}, false, accessFailure(provider.InvalidResponse, "domestic CNKI abstract page index overflowed")
		}
	}
	return domain.ArticleRedirect{}, false, nil
}

// resolveCnkiPage inspects detail only for exact normalized summary titles.
func (access *CnkiArticleAccess) resolveCnkiPage(ctx context.Context, page cnki.IssueArticlePage, article domain.ArticleLocator) (domain.ArticleRedirect, bool, error) {
	for index, summary := range page.Articles {
		title := providerText(providerField(summary, "title"))
		if title == nil || domain.NormalizeBibliographicText(*title) != domain.NormalizeBibliographicText(article.Title) {
			continue
		}
		redirect, found, err := access.resolveCnkiSummary(ctx, summary, index, article)
		if found || err != nil {
			return redirect, found, err
		}
	}
	return domain.ArticleRedirect{}, false, nil
}

// resolveCnkiSummary skips only permanent misses or incompatible detail metadata before resolving a destination.
func (access *CnkiArticleAccess) resolveCnkiSummary(ctx context.Context, summary any, ordinal int, article domain.ArticleLocator) (domain.ArticleRedirect, bool, error) {
	url := providerText(providerField(summary, "article_url"))
	if url == nil {
		return domain.ArticleRedirect{}, false, accessFailure(provider.InvalidResponse, "domestic CNKI matching article summary omitted its URL")
	}
	detail, err := access.transport.ArticleDetail(ctx, *url, providerText(providerField(summary, "platform_id")))
	if err != nil {
		if isPermanentCnkiArticleError(err) {
			logMissingCnkiArticle(ctx, ordinal, err)
			return domain.ArticleRedirect{}, false, nil
		}
		return domain.ArticleRedirect{}, false, mapCnkiProviderError(err)
	}
	if !cnkiDetailMatchesLocator(detail, article) {
		return domain.ArticleRedirect{}, false, nil
	}
	redirect, err := cnkiAbstractDestination(detail)
	return redirect, true, err
}

// cnkiAbstractDestination prefers a present permalink and enforces the exact domestic destination rules.
func cnkiAbstractDestination(detail any) (domain.ArticleRedirect, error) {
	location := providerText(providerField(detail, "permalink"))
	if location == nil {
		location = providerText(providerField(detail, "article_url"))
	}
	if location == nil {
		return domain.ArticleRedirect{}, accessFailure(provider.InvalidResponse, "domestic CNKI detail response omitted its request-time destination")
	}
	if !(strings.HasPrefix(*location, "https://navi.cnki.net/") || strings.HasPrefix(*location, "https://kns.cnki.net/") || strings.HasPrefix(*location, "https://www.cnki.net/")) {
		return domain.ArticleRedirect{}, accessFailure(provider.InvalidResponse, "domestic CNKI abstract destination is outside the allowlist")
	}
	if strings.Contains(asciiLowerSource(*location), "oversea.cnki.net") {
		return domain.ArticleRedirect{}, accessFailure(provider.InvalidResponse, "domestic CNKI abstract destination used overseas host")
	}
	return domain.ArticleRedirect{Location: *location}, nil
}

// logMissingCnkiArticle publishes the original safe ordinal and optional HTTP status classification.
func logMissingCnkiArticle(ctx context.Context, ordinal int, err error) {
	var failure *cnki.Error
	errors.As(err, &failure)
	status, hasStatus := failure.HttpStatus()
	slog.WarnContext(ctx, "index.provider.article.skipped", "event", "index.provider.article.skipped", "component", "index", "provider", CnkiProviderName, "reason", "permanent_missing", "article_ordinal", ordinal+1, "http_status", status, "has_http_status", hasStatus)
}
