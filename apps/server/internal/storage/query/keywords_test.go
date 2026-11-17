package query

import (
	"context"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
)

// TestKeywordSearchMatchesSeparatedWords covers article defaults and weekly filtering.
func TestKeywordSearchMatchesSeparatedWords(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`UPDATE articles SET title='Passion for goal setting and orientation',abstract_text='' WHERE article_id=1001;
UPDATE articles SET title='Goal orientation',abstract_text='' WHERE article_id=1012;`); err != nil {
		t.Fatal(err)
	}
	rebuildQueryFixtureSearch(t, database)
	text := "goal orientation passion"
	params := DefaultArticleListParams()
	params.Query = &text
	page, err := ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 1 || page.Items[0].ArticleID != 1001 {
		t.Fatalf("keyword search: %#v %v", page, err)
	}
	writeQueryManifest(t, configuration, "catalog.sqlite", []int64{1001})
	weeklyPage, err := WeeklyArticles(context.Background(), configuration, WeeklyArticlePageParams{
		WindowEnd: "2026-10-04T00:00:00Z", DBName: "catalog.sqlite", JournalID: 1, Limit: 10, Query: &text,
	}, nil)
	if err != nil || len(weeklyPage.Items) != 1 || weeklyPage.Items[0].ArticleID != 1001 {
		t.Fatalf("weekly keyword search: %#v %v", weeklyPage, err)
	}
	text = `"goal orientation passion"`
	params.SearchMode = domain.SearchAdvanced
	page, err = ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 0 {
		t.Fatalf("exact phrase must retain adjacency: %#v %v", page, err)
	}
}
