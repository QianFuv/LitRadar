package sources

import (
	"context"
	"errors"
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

func (index *CnkiIndexProvider) fetchBatch(ctx context.Context, catalog domain.JournalCatalogEntry, fetch domain.IndexFetchContext, attempts *[]scholarly.Attempt, cache **cnkiPageCache) (domain.ProviderBatch, error) {
	var base *string
	if fetch.CommittedAnchor != nil {
		anchor, err := decodeCnkiAnchor(*fetch.CommittedAnchor)
		if err != nil {
			return domain.ProviderBatch{}, err
		}
		base = &anchor.YearIssueId
	}
	var resume *cnki.Checkpoint
	if fetch.TraversalCheckpoint != nil {
		var err error
		resume, err = decodeCnkiCheckpoint(*fetch.TraversalCheckpoint)
		if err != nil {
			return domain.ProviderBatch{}, err
		}
		if resume.Version != cnki.CheckpointVersion || resume.BaseAnchorIssueId != nil && !cnki.IsStableIssueId(*resume.BaseAnchorIssueId) || !cnki.IsStableIssueId(resume.CandidateHeadIssueId) || !cnki.IsStableIssueId(resume.CurrentIssueId) {
			return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI checkpoint version or issue id is invalid")
		}
		if !reflect.DeepEqual(resume.BaseAnchorIssueId, base) {
			return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI checkpoint does not match the frozen committed anchor")
		}
	}
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
			return domain.ProviderBatch{}, mapCnkiProviderError(err)
		}
		if journal == nil {
			return domain.ProviderBatch{}, accessFailure(provider.NotFound, "domestic CNKI provider could not resolve the journal")
		}
		issues, err := index.client.YearIssues(ctx, journal)
		if err != nil {
			return domain.ProviderBatch{}, mapCnkiProviderError(err)
		}
		snapshot = cnkiJournalSnapshot{copyCnkiJson(journal), copyCnkiJson(issues).([]any)}
		index.snapshots[catalog.CatalogId] = snapshot
	}
	if len(snapshot.issues) == 0 {
		if resume != nil {
			return domain.ProviderBatch{}, missingCnkiCheckpointIssue()
		}
		return domain.BatchFromArticles(catalog, nil, domain.ProviderProgress{State: domain.Complete}), nil
	}
	ids := make([]string, len(snapshot.issues))
	seen := map[string]bool{}
	for ordinal, issue := range snapshot.issues {
		id := providerText(providerField(issue, "year_issue_id"))
		if id == nil || !cnki.IsStableIssueId(*id) {
			return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI issue payload omitted its stable year_issue_id")
		}
		ids[ordinal] = *id
	}
	for _, id := range ids {
		if seen[id] {
			return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI issue tree contains duplicate stable issue ids")
		}
		seen[id] = true
	}
	head := ids[0]
	if resume != nil {
		head = resume.CandidateHeadIssueId
	}
	start := slices.Index(ids, head)
	if start < 0 {
		return domain.ProviderBatch{}, missingCnkiCheckpointIssue()
	}
	end := len(ids) - 1
	if fetch.Mode == domain.Incremental && base != nil {
		if offset := slices.Index(ids[start:], *base); offset >= 0 {
			end = start + offset
		}
	}
	current, pageIndex := start, uint64(0)
	if resume != nil {
		current = slices.Index(ids, resume.CurrentIssueId)
		pageIndex = resume.PageIndex
		if current < start || current > end {
			return domain.ProviderBatch{}, missingCnkiCheckpointIssue()
		}
	}
	payload := snapshot.issues[current]
	if cnkiInteger(providerField(payload, "year")) == nil {
		return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI issue payload omitted its publication year")
	}
	issue := cnkiIssueDraft(catalog, payload)
	page, err := index.client.IssueArticles(ctx, snapshot.journal, payload, pageIndex)
	if err != nil {
		return domain.ProviderBatch{}, mapCnkiProviderError(err)
	}
	if err := validateCnkiIssuePage(page, pageIndex); err != nil {
		return domain.ProviderBatch{}, err
	}
	if *cache == nil || (*cache).catalogId != catalog.CatalogId || (*cache).issueId != ids[current] || !reflect.DeepEqual((*cache).page, page) {
		ownedPage := page
		ownedPage.Articles = copyCnkiJson(page.Articles).([]any)
		*cache = &cnkiPageCache{catalog.CatalogId, ids[current], ownedPage, map[int]*domain.ArticleDraft{}}
	}
	tasks := []cnkiDetailTask{}
	for ordinal, summary := range page.Articles {
		if _, exists := (*cache).articles[ordinal]; exists {
			continue
		}
		url := providerText(providerField(summary, "article_url"))
		if url == nil {
			return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI article summary omitted its URL")
		}
		tasks = append(tasks, cnkiDetailTask{ordinal, summary, *url, providerText(providerField(summary, "platform_id"))})
	}
	outcomes, err := index.pool.execute(ctx, tasks)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	for _, outcome := range outcomes {
		*attempts = append(*attempts, outcome.attempts...)
	}
	var firstError error
	for _, outcome := range outcomes {
		ordinal := outcome.task.ordinal
		if outcome.err != nil {
			if isPermanentCnkiArticleError(outcome.err) {
				var failure *cnki.Error
				errors.As(outcome.err, &failure)
				status, hasStatus := failure.HttpStatus()
				slog.WarnContext(ctx, "index.provider.article.skipped", "event", "index.provider.article.skipped", "component", "index", "provider", CnkiProviderName, "reason", "permanent_missing", "article_ordinal", ordinal+1, "http_status", status, "has_http_status", hasStatus)
				continue
			}
			if firstError == nil {
				firstError = mapCnkiProviderError(outcome.err)
			}
			continue
		}
		if cnkiLacksAuthorsAndDoi(outcome.task.summary, outcome.detail) {
			(*cache).articles[ordinal] = nil
			slog.InfoContext(ctx, "index.provider.article.skipped", "event", "index.provider.article.skipped", "component", "index", "provider", CnkiProviderName, "reason", "missing_authors_and_doi", "article_ordinal", ordinal+1)
			continue
		}
		article := cnkiArticleDraft(catalog, issue, outcome.task.summary, outcome.detail)
		if article == nil {
			if firstError == nil {
				firstError = accessFailure(provider.InvalidResponse, "domestic CNKI article payload could not be converted")
			}
			continue
		}
		(*cache).articles[ordinal] = article
	}
	if firstError != nil {
		return domain.ProviderBatch{}, firstError
	}
	articles := []domain.ArticleDraft{}
	for ordinal := range page.Articles {
		if article := (*cache).articles[ordinal]; article != nil {
			articles = append(articles, *article)
		}
	}
	progress := domain.ProviderProgress{State: domain.Complete}
	if page.HasNextPage || current < end {
		next := cnki.Checkpoint{Version: cnki.CheckpointVersion, BaseAnchorIssueId: base, CandidateHeadIssueId: head}
		if page.HasNextPage {
			if pageIndex == math.MaxUint64 {
				return domain.ProviderBatch{}, accessFailure(provider.InvalidResponse, "domestic CNKI page index overflowed")
			}
			next.CurrentIssueId = ids[current]
			next.PageIndex = pageIndex + 1
		} else {
			next.CurrentIssueId = ids[current+1]
		}
		encoded, err := next.Encode()
		if err != nil {
			return domain.ProviderBatch{}, accessFailure(provider.Internal, "domestic CNKI checkpoint could not be encoded")
		}
		progress = domain.ProviderProgress{State: domain.Continue, Checkpoint: &encoded}
	} else {
		encoded, err := (cnki.Anchor{Version: cnki.AnchorVersion, YearIssueId: head}).Encode()
		if err != nil {
			return domain.ProviderBatch{}, accessFailure(provider.Internal, "domestic CNKI anchor could not be encoded")
		}
		progress.NextAnchor = &encoded
	}
	batch := domain.BatchFromArticles(catalog, articles, progress)
	batch.Issues = []domain.IssueDraft{issue}
	return batch, nil
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
