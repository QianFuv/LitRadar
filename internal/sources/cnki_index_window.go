package sources

import (
	"context"
	"log/slog"
	"math"
	"reflect"
	"slices"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
	"github.com/QianFuv/LitRadar/internal/sources/cnki"
	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
)

func validateCnkiIssuePage(page cnki.IssueArticlePage, expected uint64) error {
	if page.PageIndex != expected || page.ArticleCount != uint64(len(page.Articles)) {
		return accessFailure(provider.InvalidResponse, "domestic CNKI issue article page metadata is inconsistent")
	}
	return nil
}

// fetchBatch preserves admission, frozen traversal and complete detail reconciliation under the caller lock.
func (index *CnkiIndexProvider) fetchBatch(ctx context.Context, catalog domain.JournalCatalogEntry, fetch domain.IndexFetchContext, attempts *[]scholarly.Attempt, cache **cnkiPageCache) (domain.ProviderBatch, error) {
	base, resume, err := admitCnkiTraversal(fetch)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	snapshot, err := index.cnkiSnapshot(ctx, catalog)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if len(snapshot.issues) == 0 {
		if resume != nil {
			return domain.ProviderBatch{}, missingCnkiCheckpointIssue()
		}
		return domain.BatchFromArticles(catalog, nil, domain.ProviderProgress{State: domain.Complete}), nil
	}
	ids, err := cnkiSnapshotIds(snapshot.issues)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	window, err := planCnkiTraversal(ids, base, resume, fetch.Mode)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	return index.fetchCnkiTraversalPage(ctx, catalog, snapshot, ids, base, window, attempts, cache)
}

// admitCnkiTraversal validates anchor then checkpoint before any snapshot or network access.
func admitCnkiTraversal(fetch domain.IndexFetchContext) (*string, *cnki.Checkpoint, error) {
	var base *string
	if fetch.CommittedAnchor != nil {
		anchor, err := decodeCnkiAnchor(*fetch.CommittedAnchor)
		if err != nil {
			return nil, nil, err
		}
		base = &anchor.YearIssueId
	}
	var resume *cnki.Checkpoint
	if fetch.TraversalCheckpoint != nil {
		var err error
		resume, err = decodeCnkiCheckpoint(*fetch.TraversalCheckpoint)
		if err != nil {
			return nil, nil, err
		}
		if !validCnkiResume(*resume) {
			return nil, nil, accessFailure(provider.InvalidResponse, "domestic CNKI checkpoint version or issue id is invalid")
		}
		if !reflect.DeepEqual(resume.BaseAnchorIssueId, base) {
			return nil, nil, accessFailure(provider.InvalidResponse, "domestic CNKI checkpoint does not match the frozen committed anchor")
		}
	}
	return base, resume, nil
}

// validCnkiResume admits the current checkpoint version and each supplied stable issue identity.
func validCnkiResume(resume cnki.Checkpoint) bool {
	return resume.Version == cnki.CheckpointVersion && (resume.BaseAnchorIssueId == nil || cnki.IsStableIssueId(*resume.BaseAnchorIssueId)) && cnki.IsStableIssueId(resume.CandidateHeadIssueId) && cnki.IsStableIssueId(resume.CurrentIssueId)
}

// cnkiSnapshot publishes an owned issue tree before later traversal validation.
func (index *CnkiIndexProvider) cnkiSnapshot(ctx context.Context, catalog domain.JournalCatalogEntry) (cnkiJournalSnapshot, error) {
	snapshot, exists := index.snapshots[catalog.CatalogId]
	if !exists {
		issns := []string{}
		for _, value := range []*string{catalog.Issn, catalog.Eissn} {
			if value != nil {
				issns = append(issns, *value)
			}
		}
		issns = append(issns, catalog.AllIssns...)
		journal, err := index.client.ResolveJournal(ctx, cnki.NewJournalLocator(append([]string{catalog.Title}, catalog.TitleAliases...), issns))
		if err != nil {
			return cnkiJournalSnapshot{}, mapCnkiProviderError(err)
		}
		if journal == nil {
			return cnkiJournalSnapshot{}, accessFailure(provider.NotFound, "domestic CNKI provider could not resolve the journal")
		}
		issues, err := index.client.YearIssues(ctx, journal)
		if err != nil {
			return cnkiJournalSnapshot{}, mapCnkiProviderError(err)
		}
		snapshot = cnkiJournalSnapshot{copyCnkiJson(journal), copyCnkiJson(issues).([]any)}
		index.snapshots[catalog.CatalogId] = snapshot
	}
	return snapshot, nil
}

// cnkiSnapshotIds validates every stable identifier before checking any duplicates.
func cnkiSnapshotIds(issues []any) ([]string, error) {
	ids := make([]string, len(issues))
	seen := map[string]bool{}
	for ordinal, issue := range issues {
		id := providerText(providerField(issue, "year_issue_id"))
		if id == nil || !cnki.IsStableIssueId(*id) {
			return nil, accessFailure(provider.InvalidResponse, "domestic CNKI issue payload omitted its stable year_issue_id")
		}
		ids[ordinal] = *id
	}
	for _, id := range ids {
		if seen[id] {
			return nil, accessFailure(provider.InvalidResponse, "domestic CNKI issue tree contains duplicate stable issue ids")
		}
		seen[id] = true
	}
	return ids, nil
}

// cnkiTraversalWindow retains the frozen head and source page positions for one batch.
type cnkiTraversalWindow struct {
	head         string
	current, end int
	pageIndex    uint64
}

// planCnkiTraversal selects the confirmed current issue inside the frozen head-to-base range.
func planCnkiTraversal(ids []string, base *string, resume *cnki.Checkpoint, mode domain.IndexSyncMode) (cnkiTraversalWindow, error) {
	head := ids[0]
	if resume != nil {
		head = resume.CandidateHeadIssueId
	}
	start := slices.Index(ids, head)
	if start < 0 {
		return cnkiTraversalWindow{}, missingCnkiCheckpointIssue()
	}
	end := cnkiTraversalEnd(ids, start, base, mode)
	current, pageIndex := start, uint64(0)
	if resume != nil {
		current = slices.Index(ids, resume.CurrentIssueId)
		pageIndex = resume.PageIndex
		if current < start || current > end {
			return cnkiTraversalWindow{}, missingCnkiCheckpointIssue()
		}
	}
	return cnkiTraversalWindow{head, current, end, pageIndex}, nil
}

// cnkiTraversalEnd includes an incremental base only when found at or after the frozen head.
func cnkiTraversalEnd(ids []string, start int, base *string, mode domain.IndexSyncMode) int {
	end := len(ids) - 1
	if mode == domain.Incremental && base != nil {
		if offset := slices.Index(ids[start:], *base); offset >= 0 {
			end = start + offset
		}
	}
	return end
}

// fetchCnkiTraversalPage admits page metadata and all task URLs before submitting detail work.
func (index *CnkiIndexProvider) fetchCnkiTraversalPage(ctx context.Context, catalog domain.JournalCatalogEntry, snapshot cnkiJournalSnapshot, ids []string, base *string, window cnkiTraversalWindow, attempts *[]scholarly.Attempt, cache **cnkiPageCache) (domain.ProviderBatch, error) {
	payload := snapshot.issues[window.current]
	if cnkiInteger(providerField(payload, "year")) == nil {
		return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI issue payload omitted its publication year")
	}
	issue := cnkiIssueDraft(catalog, payload)
	page, err := index.client.IssueArticles(ctx, snapshot.journal, payload, window.pageIndex)
	if err != nil {
		return domain.ProviderBatch{}, mapCnkiProviderError(err)
	}
	if err := validateCnkiIssuePage(page, window.pageIndex); err != nil {
		return domain.ProviderBatch{}, err
	}
	retainCnkiPage(cache, catalog.CatalogId, ids[window.current], page)
	tasks, err := prepareCnkiDetailTasks(page, *cache)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	outcomes, err := index.pool.execute(ctx, tasks)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	if err := acceptCnkiDetailOutcomes(ctx, catalog, issue, outcomes, *cache, attempts); err != nil {
		return domain.ProviderBatch{}, err
	}
	articles := cachedCnkiArticles(page, *cache)
	progress, err := cnkiTraversalProgress(page, ids, base, window)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	batch := domain.BatchFromArticles(catalog, articles, progress)
	batch.Issues = []domain.IssueDraft{issue}
	return batch, nil
}

// retainCnkiPage owns an article copy so in-place upstream mutation invalidates prior successes.
func retainCnkiPage(cache **cnkiPageCache, catalogId, issueId string, page cnki.IssueArticlePage) {
	if *cache == nil || (*cache).catalogId != catalogId || (*cache).issueId != issueId || !reflect.DeepEqual((*cache).page, page) {
		ownedPage := page
		ownedPage.Articles = copyCnkiJson(page.Articles).([]any)
		*cache = &cnkiPageCache{catalogId, issueId, ownedPage, map[int]*domain.ArticleDraft{}}
	}
}

// prepareCnkiDetailTasks validates every uncached summary before any pool submission.
func prepareCnkiDetailTasks(page cnki.IssueArticlePage, cache *cnkiPageCache) ([]cnkiDetailTask, error) {
	tasks := []cnkiDetailTask{}
	for ordinal, summary := range page.Articles {
		if _, exists := cache.articles[ordinal]; exists {
			continue
		}
		url := providerText(providerField(summary, "article_url"))
		if url == nil {
			return nil, accessFailure(provider.InvalidResponse, "domestic CNKI article summary omitted its URL")
		}
		tasks = append(tasks, cnkiDetailTask{ordinal, summary, *url, providerText(providerField(summary, "platform_id"))})
	}
	return tasks, nil
}

// acceptCnkiDetailOutcomes records all attempts then retains later successes after the first ordinal error.
func acceptCnkiDetailOutcomes(ctx context.Context, catalog domain.JournalCatalogEntry, issue domain.IssueDraft, outcomes []cnkiDetailOutcome, cache *cnkiPageCache, attempts *[]scholarly.Attempt) error {
	for _, outcome := range outcomes {
		*attempts = append(*attempts, outcome.attempts...)
	}
	var firstError error
	for _, outcome := range outcomes {
		if err := acceptCnkiDetailOutcome(ctx, catalog, issue, outcome, cache); err != nil && firstError == nil {
			firstError = err
			if outcome.err != nil {
				firstError = mapCnkiProviderError(err)
			}
		}
	}
	return firstError
}

// acceptCnkiDetailOutcome leaves permanent misses uncached, caches filtered nils and admits converted articles.
func acceptCnkiDetailOutcome(ctx context.Context, catalog domain.JournalCatalogEntry, issue domain.IssueDraft, outcome cnkiDetailOutcome, cache *cnkiPageCache) error {
	ordinal := outcome.task.ordinal
	if outcome.err != nil {
		if isPermanentCnkiArticleError(outcome.err) {
			logMissingCnkiArticle(ctx, ordinal, outcome.err)
			return nil
		}
		return outcome.err
	}
	if cnkiLacksAuthorsAndDoi(outcome.task.summary, outcome.detail) {
		cache.articles[ordinal] = nil
		slog.InfoContext(ctx, "index.provider.article.skipped", "event", "index.provider.article.skipped", "component", "index", "provider", CnkiProviderName, "reason", "missing_authors_and_doi", "article_ordinal", ordinal+1)
		return nil
	}
	article := cnkiArticleDraft(catalog, issue, outcome.task.summary, outcome.detail)
	if article == nil {
		return accessFailure(provider.InvalidResponse, "domestic CNKI article payload could not be converted")
	}
	cache.articles[ordinal] = article
	return nil
}

// cachedCnkiArticles preserves source ordinal order and excludes cached filtered entries.
func cachedCnkiArticles(page cnki.IssueArticlePage, cache *cnkiPageCache) []domain.ArticleDraft {
	articles := []domain.ArticleDraft{}
	for ordinal := range page.Articles {
		if article := cache.articles[ordinal]; article != nil {
			articles = append(articles, *article)
		}
	}
	return articles
}

// cnkiTraversalProgress encodes continuation or committed head only after successful detail reconciliation.
func cnkiTraversalProgress(page cnki.IssueArticlePage, ids []string, base *string, window cnkiTraversalWindow) (domain.ProviderProgress, error) {
	head, current, end, pageIndex := window.head, window.current, window.end, window.pageIndex
	progress := domain.ProviderProgress{State: domain.Complete}
	if page.HasNextPage || current < end {
		next := cnki.Checkpoint{Version: cnki.CheckpointVersion, BaseAnchorIssueId: base, CandidateHeadIssueId: head}
		if page.HasNextPage {
			if pageIndex == math.MaxUint64 {
				return domain.ProviderProgress{}, accessFailure(provider.InvalidResponse, "domestic CNKI page index overflowed")
			}
			next.CurrentIssueId = ids[current]
			next.PageIndex = pageIndex + 1
		} else {
			next.CurrentIssueId = ids[current+1]
		}
		encoded, err := next.Encode()
		if err != nil {
			return domain.ProviderProgress{}, accessFailure(provider.Internal, "domestic CNKI checkpoint could not be encoded")
		}
		progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: &encoded}
	} else {
		encoded, err := (cnki.Anchor{Version: cnki.AnchorVersion, YearIssueId: head}).Encode()
		if err != nil {
			return domain.ProviderProgress{}, accessFailure(provider.Internal, "domestic CNKI anchor could not be encoded")
		}
		progress.NextAnchor = &encoded
	}
	return progress, nil
}

func copyCnkiJson(value any) any {
	switch value := value.(type) {
	case map[string]any:
		copy := make(map[string]any, len(value))
		for key, item := range value {
			copy[key] = copyCnkiJson(item)
		}
		return copy
	case []any:
		if value == nil {
			return []any(nil)
		}
		copy := make([]any, len(value))
		for index, item := range value {
			copy[index] = copyCnkiJson(item)
		}
		return copy
	default:
		return value
	}
}
