package scholarly

import (
	"context"
	"errors"
	"log/slog"
	"strings"
	"sync"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
)

// IndexProvider serializes complete source batches and retains only disposable traversal state.
type IndexProvider struct {
	mutex                 sync.Mutex
	client                *Client
	hasSemanticScholarKey bool
	worksetDir            string
	now                   func() time.Time
}

// NewIndexProvider binds a source transport to a core-owned absolute workset directory.
func NewIndexProvider(transport Transport, hasSemanticScholarKey bool, worksetDir string) *IndexProvider {
	return &IndexProvider{client: NewClient(transport, hasSemanticScholarKey), hasSemanticScholarKey: hasSemanticScholarKey, worksetDir: worksetDir, now: time.Now}
}

// NewIndexRegistration declares the scholarly indexing capability independently of online access.
func NewIndexRegistration(transport Transport, hasSemanticScholarKey bool, worksetDir string) (*provider.Registration, error) {
	return provider.NewRegistration(provider.Descriptor{Name: "scholarly", Capabilities: provider.Capabilities{IndexContent: true}}, provider.Implementations{IndexContent: NewIndexProvider(transport, hasSemanticScholarKey, worksetDir)})
}

// Fetch returns one canonical batch and drains its attempt accounting on every outcome.
func (index *IndexProvider) Fetch(ctx context.Context, catalog domain.JournalCatalogEntry, fetch domain.IndexFetchContext) (domain.ProviderBatch, error) {
	index.mutex.Lock()
	defer index.mutex.Unlock()
	defer func() {
		attempts := index.client.DrainAttempts()
		failures, retries := 0, 0
		for _, attempt := range attempts {
			if !attempt.DidSucceed {
				failures++
			}
			if attempt.DidRetry {
				retries++
			}
		}
		slog.InfoContext(ctx, "index.provider.attempts", "event", "index.provider.attempts", "component", "index", "provider", "scholarly", "attempt_count", len(attempts), "failure_count", failures, "retry_count", retries)
	}()
	window, source, err := indexWindowFromContext(fetch, indexCurrentDate(index.now()))
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if source != nil {
		switch source.Kind {
		case "crossref_workset":
			batch, err := index.fetchWorkset(ctx, catalog, window, *source.State)
			if err == nil || err.Error() != "Crossref workset database is damaged" {
				return batch, err
			}
			scope, err := indexWorksetScope(catalog, window)
			if err != nil {
				return domain.ProviderBatch{}, err
			}
			cache, recovered, err := OpenCrossrefWorkset(index.worksetDir, scope, *source.State)
			if err != nil {
				return domain.ProviderBatch{}, err
			}
			defer cache.Close()
			next := cache.Checkpoint()
			if !recovered {
				next, err = cache.Recollect()
				if err != nil {
					return domain.ProviderBatch{}, err
				}
			}
			return replayedCrossrefBatch(catalog, window, next)
		case "crossref":
			prepareIndexReplay(&window)
			frozen, err := index.frozenEpoch()
			if err != nil {
				return domain.ProviderBatch{}, err
			}
			return index.startWorkset(catalog, window, source.Issn, frozen, nil)
		case "open_alex":
			return index.fetchOpenAlex(ctx, catalog, window, source)
		}
	}
	issns := domain.CatalogIssns(catalog)
	if len(issns) > 0 {
		frozen, err := index.frozenEpoch()
		if err != nil {
			return domain.ProviderBatch{}, err
		}
		return index.startWorkset(catalog, window, issns[0], frozen, nil)
	}
	return index.fetchOpenAlex(ctx, catalog, window, nil)
}

func indexCurrentDate(now time.Time) *string {
	if now.Unix() < 0 {
		return nil
	}
	return indexDate(now.Unix())
}
func indexDate(epoch int64) *string {
	value, err := crossrefTimestamp(epoch)
	if err != nil {
		return nil
	}
	date, _, _ := strings.Cut(value, "T")
	return &date
}
func (index *IndexProvider) frozenEpoch() (int64, error) {
	epoch := index.now().Unix()
	if epoch < 0 {
		return 0, &provider.Error{Kind: provider.Internal, Message: "system clock is before the Unix epoch"}
	}
	return epoch, nil
}

func mapIndexSourceError(err error) error {
	kind := provider.TemporarilyUnavailable
	var failure *Error
	if errors.As(err, &failure) {
		switch failure.Kind {
		case "HttpStatus":
			if failure.StatusCode == 404 {
				kind = provider.NotFound
			}
		case "InvalidFixture":
			kind = provider.InvalidResponse
		case "Configuration":
			kind = provider.Internal
		}
	}
	return &provider.Error{Kind: kind, Message: "scholarly provider request failed"}
}

func continueIndexBatch(catalog domain.JournalCatalogEntry, window indexWindow, source indexSource, articles []domain.ArticleDraft) (domain.ProviderBatch, error) {
	checkpoint, err := encodeIndexCheckpoint(indexCheckpoint{Version: 2, Window: window, Source: source})
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	return domain.BatchFromArticles(catalog, articles, domain.ProviderProgress{State: domain.Continue, Checkpoint: &checkpoint}), nil
}
func completeIndexBatch(catalog domain.JournalCatalogEntry, window indexWindow, articles []domain.ArticleDraft) (domain.ProviderBatch, error) {
	anchor := window.CandidateAnchor
	if anchor == nil {
		anchor = window.BaseAnchor
	}
	var encoded *string
	if anchor != nil {
		body, err := anchor.MarshalJSON()
		if err != nil {
			return domain.ProviderBatch{}, &provider.Error{Kind: provider.Internal, Message: "scholarly anchor could not be encoded"}
		}
		encoded = new(string(body))
	}
	return domain.BatchFromArticles(catalog, articles, domain.ProviderProgress{State: domain.Complete, NextAnchor: encoded}), nil
}

func (index *IndexProvider) enrichCrossref(ctx context.Context, catalog domain.JournalCatalogEntry, works []any) ([]domain.ArticleDraft, error) {
	dois := []string{}
	for _, work := range works {
		if doi := textDoi(field(work, "DOI")); doi != nil {
			dois = append(dois, *doi)
		}
	}
	semantic := map[string]any{}
	var err error
	if len(dois) > 0 && index.hasSemanticScholarKey {
		semantic, err = index.client.FetchSemanticScholarByDois(ctx, dois, 500)
		if err != nil {
			return nil, mapIndexSourceError(err)
		}
	}
	openAlexDois := []string{}
	for _, work := range works {
		doi := textDoi(field(work, "DOI"))
		if doi == nil {
			continue
		}
		hasTitle := firstText(field(work, "title")) != nil
		hasAbstract := false
		if text := jsonText(field(work, "abstract")); text != nil {
			hasAbstract = stripMarkup(*text) != nil
		}
		hasAccess := strictBool(field(semantic[*doi], "isOpenAccess")) != nil
		if !(hasTitle && hasAbstract && hasAccess) {
			openAlexDois = append(openAlexDois, *doi)
		}
	}
	openAlex := map[string]any{}
	if len(openAlexDois) > 0 {
		openAlex, err = index.client.FetchOpenAlexByDois(ctx, openAlexDois, 100)
		if err != nil {
			return nil, mapIndexSourceError(err)
		}
	}
	articles := []domain.ArticleDraft{}
	for _, work := range works {
		var oa, ss any
		if doi := textDoi(field(work, "DOI")); doi != nil {
			oa, ss = openAlex[*doi], semantic[*doi]
		}
		if article := crossrefArticle(catalog, work, oa, ss); article != nil {
			articles = append(articles, *article)
		}
	}
	return articles, nil
}
