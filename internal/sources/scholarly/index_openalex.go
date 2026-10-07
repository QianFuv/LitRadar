package scholarly

import (
	"context"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
)

func (index *IndexProvider) fetchOpenAlex(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, source *indexSource) (domain.ProviderBatch, error) {
	if source == nil {
		var err error
		source, err = index.resolveOpenAlexSource(ctx, catalog)
		if err != nil {
			return domain.ProviderBatch{}, err
		}
	}
	if source.Kind != "open_alex" {
		return domain.ProviderBatch{}, &provider.Error{Kind: provider.Internal, Message: "invalid OpenAlex continuation source"}
	}
	page, err := index.client.FetchOpenAlexWorksBySourcePage(ctx, source.SourceId, indexWindowFilter(window, indexCurrentDate(index.now())), source.Cursor)
	if err != nil {
		return domain.ProviderBatch{}, mapIndexSourceError(err)
	}
	effective := *source
	if page.DidFallbackToUnfiltered {
		effective.Cursor = nil
	}
	next, err := nextOpenAlexSource(page, effective)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	articles, anchors, hasUnknown := projectOpenAlexPage(catalog, page.Items)
	if page.DidFallbackToUnfiltered {
		unboundIndexWindow(&window)
	}
	plan, err := planIndexPageWindow(window, anchors, hasUnknown, next != nil)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
	return finishOpenAlexPage(catalog, plan, articles, next, effective.SourceId)
}

// resolveOpenAlexSource tries catalog ISSNs before title and requires a normalized source identifier.
func (index *IndexProvider) resolveOpenAlexSource(ctx context.Context, catalog domain.JournalCatalogEntry) (*indexSource, error) {
	observed, err := index.client.FetchOpenAlexSourceByIssns(ctx, domain.CatalogIssns(catalog))
	if err != nil {
		return nil, mapIndexSourceError(err)
	}
	if observed == nil {
		observed, err = index.client.FetchOpenAlexSourceByTitle(ctx, catalog.Title)
		if err != nil {
			return nil, mapIndexSourceError(err)
		}
	}
	if observed == nil {
		return nil, &provider.Error{Kind: provider.NotFound, Message: "scholarly provider could not resolve the journal"}
	}
	id := jsonText(field(observed, "id"))
	if id == nil {
		return nil, invalidWorkset("OpenAlex source has no identifier")
	}
	return &indexSource{Kind: "open_alex", SourceId: *id}, nil
}

// nextOpenAlexSource rejects repeated cursors only for nonempty continuing pages.
func nextOpenAlexSource(page WorksPage, effective indexSource) (*indexSource, error) {
	var next *indexSource
	if page.NextCursor != nil && len(page.Items) > 0 {
		if effective.Cursor != nil && *effective.Cursor == *page.NextCursor {
			return nil, invalidWorkset("scholarly provider returned a repeated cursor")
		}
		next = &indexSource{Kind: "open_alex", SourceId: effective.SourceId, Cursor: clonePointer(page.NextCursor)}
	}
	return next, nil
}

// projectOpenAlexPage keeps represented articles aligned with issue anchors and unknown flags.
func projectOpenAlexPage(catalog domain.JournalCatalogEntry, items []any) ([]domain.ArticleDraft, []*Anchor, bool) {
	articles := []domain.ArticleDraft{}
	anchors := []*Anchor{}
	hasUnknown := false
	for _, work := range items {
		article := openAlexArticle(catalog, work)
		if article == nil {
			hasUnknown = true
			continue
		}
		anchor := IssueAnchorFromFields(article.PublicationYear, article.Date, article.IssueTitle, article.Volume, article.IssueNumber)
		if anchor == nil {
			hasUnknown = true
		}
		articles = append(articles, *article)
		anchors = append(anchors, anchor)
	}
	return articles, anchors, hasUnknown
}

// finishOpenAlexPage preserves selected positions, continuation requirements and unbounded replay source.
func finishOpenAlexPage(catalog domain.JournalCatalogEntry, plan indexPagePlan, articles []domain.ArticleDraft, next *indexSource, sourceId string) (domain.ProviderBatch, error) {
	selected := make([]domain.ArticleDraft, 0, len(plan.SelectedIndices))
	for _, position := range plan.SelectedIndices {
		selected = append(selected, articles[position])
	}
	switch plan.Progress {
	case "continue":
		if next == nil {
			return domain.ProviderBatch{}, &provider.Error{Kind: provider.Internal, Message: "scholarly page plan requires a missing continuation source"}
		}
		return continueIndexBatch(catalog, plan.Window, *next, selected)
	case "complete":
		return completeIndexBatch(catalog, plan.Window, selected)
	default:
		unboundIndexWindow(&plan.Window)
		return continueIndexBatch(catalog, plan.Window, indexSource{Kind: "open_alex", SourceId: sourceId}, nil)
	}
}
