package query

import (
	"context"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
)

// TestSearchFoldingMatchesCanonicalAccents exercises article, weekly and related FTS operands.
func TestSearchFoldingMatchesCanonicalAccents(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`UPDATE articles SET title='Résumé Études',authors_json='["Émile"]' WHERE article_id=1001;
UPDATE articles SET title='RESUME ETUDES',abstract_text='ÉMILE' WHERE article_id=1012;`); err != nil {
		t.Fatal(err)
	}
	rebuildQueryFixtureSearch(t, database)
	ctx := context.Background()
	for _, scenario := range []struct {
		query string
		mode  domain.SearchMode
	}{
		{"resume", domain.SearchSimple},
		{"RÉSUMÉ", domain.SearchSimple},
		{"emile", domain.SearchSimple},
		{`title:résumé AND "ÉTUDES"`, domain.SearchAdvanced},
		{`authors:emile`, domain.SearchAdvanced},
	} {
		params := DefaultArticleListParams()
		params.Query, params.SearchMode = &scenario.query, scenario.mode
		page, err := ListArticles(ctx, configuration, nil, params)
		if err != nil || len(page.Items) == 0 {
			t.Fatalf("article query %q: %#v %v", scenario.query, page, err)
		}
		hasAccentedSource := false
		for _, article := range page.Items {
			if article.ArticleId == 1001 {
				hasAccentedSource = true
				if article.Title != "Résumé Études" {
					t.Fatalf("canonical title changed: %q", article.Title)
				}
			}
		}
		if !hasAccentedSource {
			t.Fatalf("query %q missed accented article", scenario.query)
		}
	}
	query := "RÉSUMÉ"
	writeQueryManifest(t, configuration, "catalog.sqlite", []int64{1001, 1012})
	page, err := WeeklyArticles(ctx, configuration, WeeklyArticlePageParams{
		WindowEnd: "2026-10-04T00:00:00Z", DbName: "catalog.sqlite", JournalId: 1, Limit: 10, Query: &query,
	}, nil)
	if err != nil || len(page.Items) != 1 || page.Items[0].ArticleId != 1001 {
		t.Fatalf("weekly accent search: %#v %v", page, err)
	}
	related, err := RelatedArticles(ctx, configuration, nil, 1001, 10)
	if err != nil || len(related) != 1 || related[0].ArticleId != 1012 {
		t.Fatalf("related accent search: %#v %v", related, err)
	}
}

// TestSimpleBlankProjectionKeepsSearch prevents discarded punctuation from admitting all rows.
func TestSimpleBlankProjectionKeepsSearch(t *testing.T) {
	configuration, _ := queryFixture(t)
	for _, query := range []string{"!!!", "😀", "\u0301"} {
		params := DefaultArticleListParams()
		params.Query = &query
		page, err := ListArticles(context.Background(), configuration, nil, params)
		if err != nil || len(page.Items) != 0 {
			t.Fatalf("nonblank query %q: %#v %v", query, page, err)
		}
	}
}
