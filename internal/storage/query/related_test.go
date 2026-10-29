package query

import (
	"context"
	"errors"
	"reflect"
	"strings"
	"testing"
)

// TestRelatedTermsKeepContentWordsAndHanBigrams documents which title tokens drive recall.
func TestRelatedTermsKeepContentWordsAndHanBigrams(t *testing.T) {
	cases := []struct {
		title string
		want  []string
	}{
		{"Graph Neural Networks for the Traffic of 2024", []string{"graph", "neural", "networks", "traffic", "2024"}},
		{"数字经济的研究与发展：基于实证", []string{"数字", "字经", "经济"}},
		{"农村 三维性 X", []string{"农村", "三维性"}},
		{"Of the and a", []string{}},
		{"Graph graph GRAPH", []string{"graph"}},
	}
	for _, testCase := range cases {
		if got := relatedTerms(testCase.title); !reflect.DeepEqual(got, testCase.want) {
			t.Errorf("relatedTerms(%q) = %q, want %q", testCase.title, got, testCase.want)
		}
	}
	long := relatedTerms(strings.Repeat("alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma ", 1))
	if len(long) != relatedTermLimit {
		t.Fatalf("expected %d terms, got %d", relatedTermLimit, len(long))
	}
	if got := relatedMatch([]string{"graph", `say "hi"`}); got != `{title abstract_text} : ("graph" OR "say ""hi""")` {
		t.Fatalf("unexpected match expression %q", got)
	}
}

// TestRelatedArticlesRankBySimilarityAndExcludeSource seeds titles and abstracts with known
// overlap and checks bm25 ordering, source and duplicate-title exclusion, and validation.
func TestRelatedArticlesRankBySimilarityAndExcludeSource(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`
INSERT INTO articles(article_id,journal_id,title,abstract_text,authors_json,date) VALUES
 (2001,1,'Graph neural networks for traffic forecasting','Spatiotemporal traffic prediction.','[]','2024-01-01'),
 (2002,1,'Traffic forecasting with graph neural networks','Graph models for road traffic.','[]','2020-01-01'),
 (2003,1,'Graph coloring heuristics','Combinatorial search.','[]','2025-01-01'),
 (2004,2,'Deep learning survey','We review graph neural networks for traffic forecasting.','[]','2023-01-01'),
 (2005,2,'Protein folding dynamics','Molecular simulation.','[]','2026-01-01'),
 (2006,2,'Traffic Forecasting with Graph Neural Networks.','A reprinted copy of an earlier editorial record.','[]','2021-01-01'),
 (2007,2,'Graph Neural Networks for Traffic Forecasting','Spatiotemporal traffic prediction.','[]','2022-01-01'),
 (2010,2,'Soil erosion modelling','Rainfall runoff.','[]','2020-01-01'),
 (2011,2,'Coral reef ecology','Marine biodiversity.','[]','2020-01-01'),
 (2012,2,'Medieval trade guilds','Economic history.','[]','2020-01-01'),
 (2013,2,'Glacier mass balance','Climate records.','[]','2020-01-01'),
 (2014,2,'Bird song learning','Vocal development.','[]','2020-01-01'),
 (2015,2,'Volcanic ash dispersal','Plume dynamics.','[]','2020-01-01');
INSERT INTO article_listing(article_id,journal_id,date) SELECT article_id,journal_id,date FROM articles WHERE article_id>=2001;
INSERT INTO article_search(rowid,article_id,title,abstract_text,authors,journal_title)
 SELECT article_id,article_id,title,abstract_text,'','' FROM articles WHERE article_id>=2001;`); err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	related, err := RelatedArticles(ctx, configuration, nil, 2001, 10)
	if err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, len(related))
	for index, article := range related {
		ids[index] = int64(article.ArticleId)
	}
	if len(ids) == 0 || ids[0] != 2002 {
		t.Fatalf("expected the title-overlap article first, got %v", ids)
	}
	for _, unexpected := range []int64{2001, 2005, 2006, 2007} {
		for _, id := range ids {
			if id == unexpected {
				t.Fatalf("article %d must not be related: %v", unexpected, ids)
			}
		}
	}
	if !reflect.DeepEqual(ids, []int64{2002, 2004, 2003}) {
		t.Fatalf("unexpected bm25 order %v", ids)
	}
	if related[0].Title != "Traffic forecasting with graph neural networks" {
		t.Fatalf("articles were not enriched: %+v", related[0])
	}
	limited, err := RelatedArticles(ctx, configuration, nil, 2001, 1)
	if err != nil || len(limited) != 1 || int64(limited[0].ArticleId) != 2002 {
		t.Fatalf("limit not applied: %+v, %v", limited, err)
	}
	for _, limit := range []int64{0, RelatedArticleMaxLimit + 1} {
		var invalid InvalidInput
		if _, err := RelatedArticles(ctx, configuration, nil, 2001, limit); !errors.As(err, &invalid) {
			t.Fatalf("limit %d: expected invalid input, got %v", limit, err)
		}
	}
	var missing NotFound
	if _, err := RelatedArticles(ctx, configuration, nil, 999999, 5); !errors.As(err, &missing) {
		t.Fatalf("expected not found, got %v", err)
	}
}

// TestRelatedExpressionRecallsWithRareTermsWithinBudget keeps bm25 work bounded: rare terms are
// recalled while their costs fit the budget, common Latin words still contribute to ranking,
// common Han phrases are left out, and a title of only common terms requires its rarest term
// together with any other.
func TestRelatedExpressionRecallsWithRareTermsWithinBudget(t *testing.T) {
	cases := []struct {
		terms []string
		costs []int
		want  string
	}{
		{[]string{"a", "b", "c", "d"}, []int{100, 29000, 50, 3000}, `{title abstract_text} : (("c" OR "a" OR "d") AND ("c" OR "a" OR "d" OR "b"))`},
		{[]string{"经济", "物元", "增长"}, []int{40000, 400, 40000}, `{title abstract_text} : ("物元")`},
		{[]string{"a", "b", "c"}, []int{40000, 31000, 50000}, `{title abstract_text} : ("b" AND ("a" OR "c"))`},
		{[]string{"a"}, []int{90000}, `{title abstract_text} : ("a")`},
	}
	for _, testCase := range cases {
		if got := relatedExpression(testCase.terms, testCase.costs); got != testCase.want {
			t.Errorf("relatedExpression(%q, %v) = %q, want %q", testCase.terms, testCase.costs, got, testCase.want)
		}
	}
}

// TestRelatedArticlesPreferTitleOverlapToAbstractMentions keeps title-to-title similarity ahead
// of an unrelated title whose abstract happens to mention every source title word.
func TestRelatedArticlesPreferTitleOverlapToAbstractMentions(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`
INSERT INTO articles(article_id,journal_id,title,abstract_text,authors_json,date) VALUES
 (3001,1,'Graph neural networks for traffic forecasting','Spatiotemporal prediction.','[]','2024-01-01'),
 (3002,1,'Graph neural networks benchmarks','Evaluation protocols.','[]','2024-01-01'),
 (3003,2,'A survey of learning methods','We review graph neural networks for traffic forecasting.','[]','2024-01-01'),
 (3010,2,'Soil erosion modelling','Rainfall runoff.','[]','2020-01-01'),
 (3011,2,'Coral reef ecology','Marine biodiversity.','[]','2020-01-01'),
 (3012,2,'Medieval trade guilds','Economic history.','[]','2020-01-01'),
 (3013,2,'Glacier mass balance','Climate records.','[]','2020-01-01');
INSERT INTO article_listing(article_id,journal_id,date) SELECT article_id,journal_id,date FROM articles WHERE article_id>=3001;
INSERT INTO article_search(rowid,article_id,title,abstract_text,authors,journal_title)
 SELECT article_id,article_id,title,abstract_text,'','' FROM articles WHERE article_id>=3001;`); err != nil {
		t.Fatal(err)
	}
	related, err := RelatedArticles(context.Background(), configuration, nil, 3001, 10)
	if err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, len(related))
	for index, article := range related {
		ids[index] = int64(article.ArticleId)
	}
	if !reflect.DeepEqual(ids, []int64{3002, 3003}) {
		t.Fatalf("expected title overlap before abstract mentions, got %v", ids)
	}
}
