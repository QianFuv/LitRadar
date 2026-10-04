package delivery

import (
	"context"
	"slices"
	"strconv"

	article "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/recommend"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/query"
)

type deliveryInput struct {
	issueKeys, inpressKeys []string
	articleIds             []int64
}

func (input deliveryInput) isEmpty() bool {
	return len(input.issueKeys)+len(input.inpressKeys)+len(input.articleIds) == 0
}

func (input deliveryInput) items() []store.RunItemCreate {
	items := []store.RunItemCreate{}
	for _, key := range input.issueKeys {
		items = append(items, store.RunItemCreate{ItemKind: store.ItemKindIssue, ItemKey: key})
	}
	for _, key := range input.inpressKeys {
		items = append(items, store.RunItemCreate{ItemKind: store.ItemKindInPress, ItemKey: key})
	}
	for _, id := range input.articleIds {
		items = append(items, store.RunItemCreate{ItemKind: store.ItemKindArticle, ItemKey: strconv.FormatInt(id, 10), ArticleId: pointer(id)})
	}
	return items
}

func inputFromSource(manifest *recommend.ChangeManifest, previous, current recommend.Snapshot) deliveryInput {
	if manifest != nil {
		return deliveryInput{manifest.PendingIssueKeys, manifest.PendingInpressKeys, manifest.PendingArticleIds}
	}
	return deliveryInput{recommend.ComputeChangedIssueKeys(previous.IssueArticleCounts, current.IssueArticleCounts), recommend.ComputeChangedInpressKeys(previous.InpressArticleCounts, current.InpressArticleCounts), nil}
}

func inputFromItems(items []store.RunItemRecord) deliveryInput {
	input := deliveryInput{}
	for _, item := range items {
		switch item.ItemKind {
		case store.ItemKindIssue:
			input.issueKeys = append(input.issueKeys, item.ItemKey)
		case store.ItemKindInPress:
			input.inpressKeys = append(input.inpressKeys, item.ItemKey)
		case store.ItemKindArticle:
			if item.ArticleId != nil {
				input.articleIds = append(input.articleIds, *item.ArticleId)
			}
		}
	}
	slices.Sort(input.issueKeys)
	input.issueKeys = slices.Compact(input.issueKeys)
	slices.Sort(input.inpressKeys)
	input.inpressKeys = slices.Compact(input.inpressKeys)
	slices.Sort(input.articleIds)
	input.articleIds = slices.Compact(input.articleIds)
	return input
}

func loadCandidates(ctx context.Context, filename string, input deliveryInput) ([]article.ArticleCandidate, error) {
	if len(input.articleIds) > 0 {
		return query.FetchCandidatesForArticleIds(ctx, filename, input.articleIds)
	}
	candidates, err := query.FetchCandidatesForIssueKeys(ctx, filename, input.issueKeys)
	if err != nil {
		return nil, err
	}
	inpress, err := query.FetchCandidatesForInPressKeys(ctx, filename, input.inpressKeys)
	return append(candidates, inpress...), err
}

func (engine *deliveryEngine) finalizeProgress(ctx context.Context, run *durableRun) error {
	if err := run.renew(ctx, engine.repository, unixNow()); err != nil {
		return err
	}
	items, err := engine.repository.ListRunItems(ctx, run.run.Id)
	if err != nil {
		return err
	}
	for _, item := range items {
		if item.ItemKind == store.ItemKindSubscriber || item.Status.IsTerminal() {
			continue
		}
		claimed, err := engine.repository.ClaimItem(ctx, run.run.Id, run.owner, run.run.Revision, item.Id, run.owner, unixNow(), deliveryLeaseSeconds)
		if err != nil {
			return err
		}
		if _, err = engine.repository.FinalizeItem(ctx, claimed.Id, run.owner, claimed.Revision, store.ItemStatusSucceeded, nil, nil, unixNow()); err != nil {
			return err
		}
	}
	return nil
}
