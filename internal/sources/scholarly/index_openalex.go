package scholarly

import (
	"context"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
)

func (index *IndexProvider) fetchOpenAlex(ctx context.Context, catalog domain.JournalCatalogEntry, window indexWindow, source *indexSource) (domain.ProviderBatch, error) {
	if source == nil {
		observed, err := index.client.FetchOpenAlexSourceByIssns(ctx, domain.CatalogIssns(catalog))
		if err != nil {
			return domain.ProviderBatch{}, mapIndexSourceError(err)
		}
		if observed == nil {
			observed, err = index.client.FetchOpenAlexSourceByTitle(ctx, catalog.Title)
			if err != nil {
				return domain.ProviderBatch{}, mapIndexSourceError(err)
			}
		}
		if observed == nil {
			return domain.ProviderBatch{}, &provider.Error{Kind: provider.NotFound, Message: "scholarly provider could not resolve the journal"}
		}
		id := jsonText(field(observed, "id"))
		if id == nil {
			return domain.ProviderBatch{}, invalidWorkset("OpenAlex source has no identifier")
		}
		source = &indexSource{Kind: "open_alex", SourceId: *id}
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
	var next *indexSource
	if page.NextCursor != nil && len(page.Items) > 0 {
		if effective.Cursor != nil && *effective.Cursor == *page.NextCursor {
			return domain.ProviderBatch{}, invalidWorkset("scholarly provider returned a repeated cursor")
		}
		next = &indexSource{Kind: "open_alex", SourceId: effective.SourceId, Cursor: clonePointer(page.NextCursor)}
	}
	articles := []domain.ArticleDraft{}
	anchors := []*Anchor{}
	hasUnknown := false
	for _, work := range page.Items {
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
	if page.DidFallbackToUnfiltered {
		unboundIndexWindow(&window)
	}
	plan, err := planIndexPageWindow(window, anchors, hasUnknown, next != nil)
	if err != nil {
		return domain.ProviderBatch{}, err
	}
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
		return continueIndexBatch(catalog, plan.Window, indexSource{Kind: "open_alex", SourceId: effective.SourceId}, nil)
	}
}
