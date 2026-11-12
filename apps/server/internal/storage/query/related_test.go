package query

import (
	"context"
	"database/sql"
	"errors"
	"github.com/QianFuv/LitRadar/apps/server/internal/storage/search"
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
		{"Ünïcode École", []string{"unicode", "ecole"}},
		{"Résumé resume RÉSUMÉ", []string{"resume"}},
		{"数字经济的研究与发展：ÉTUDES", []string{"数字", "字经", "经济", "etudes"}},
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
	if got := relatedMatch([]string{"graph", `say "hi"`}); got != `{title abstract_text} : ("graph" OR "say hi")` {
		t.Fatalf("unexpected match expression %q", got)
	}
}

// TestRelatedTermsIgnorePublisherMarkup keeps XML attributes from displacing topic terms.
func TestRelatedTermsIgnorePublisherMarkup(t *testing.T) {
	markup := `<mml:math xmlns:mml="http://www.w3.org/1998/Math/MathML" display="inline" id="d1e232" altimg="si13.svg"><mml:mi>H</mml:mi></mml:math> stability of neural networks`
	if terms := relatedTerms(markup); !reflect.DeepEqual(terms, []string{"stability", "neural", "networks"}) {
		t.Fatal("publisher attributes displaced substantive title terms", terms)
	}
	if terms := relatedTerms(`Graph <i>neural</i> networks &amp; applications`); !reflect.DeepEqual(terms, []string{"graph", "neural", "networks", "applications"}) {
		t.Fatal("inline markup did not preserve displayed words", terms)
	}
	for _, title := range []string{"Comparison x<y and z>w", "Eigenvalues < 0 and kernels > 1"} {
		if got := relatedPlainTitle(title); got != title {
			t.Fatal("plain mathematical comparison was interpreted as publisher markup", title, got)
		}
	}
}

// TestRelatedBudgetPreservesHanRecall avoids narrowing recall to accidental Han boundaries.
func TestRelatedBudgetPreservesHanRecall(t *testing.T) {
	for _, testCase := range []struct {
		terms []string
		want  int
	}{
		{[]string{"graph", "neural"}, 3000},
		{[]string{"信息", "效率"}, 30000},
		{[]string{"graph", "网络"}, 30000},
	} {
		if got := relatedBudget(testCase.terms); got != testCase.want {
			t.Fatal("wrong language-specific recall budget", testCase.terms, got)
		}
	}
	if got := relatedExpression([]string{"graph", "networks"}, []int{1000, 4000}); got != `{title abstract_text} : (("graph") AND ("graph" OR "networks"))` {
		t.Fatal("Latin recall must retain only the affordable anchor", got)
	}
	if got := relatedExpression([]string{"graph", "网络"}, []int{1000, 4000}); got != `{title abstract_text} : ("graph" OR "网 络")` {
		t.Fatal("mixed Han recall must preserve the broader anchor set", got)
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
`); err != nil {
		t.Fatal(err)
	}
	rebuildQueryFixtureSearch(t, database)
	ctx := context.Background()
	related, err := RelatedArticles(ctx, configuration, nil, 2001, 10)
	if err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, len(related))
	for index, article := range related {
		ids[index] = int64(article.ArticleID)
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
	if err != nil || len(limited) != 1 || int64(limited[0].ArticleID) != 2002 {
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
		{[]string{"a", "b", "c", "d"}, []int{100, 2900, 50, 300}, `{title abstract_text} : (("c" OR "a" OR "d") AND ("c" OR "a" OR "d" OR "b"))`},
		{[]string{"经济", "物元", "增长"}, []int{40000, 400, 40000}, `{title abstract_text} : ("物 元")`},
		{[]string{"a", "b", "c"}, []int{40000, 31000, 50000}, `{title abstract_text} : ("b" AND ("a" OR "c"))`},
		{[]string{"a"}, []int{90000}, `{title abstract_text} : ("a")`},
	}
	for _, testCase := range cases {
		if got := relatedExpression(testCase.terms, testCase.costs); got != testCase.want {
			t.Errorf("relatedExpression(%q, %v) = %q, want %q", testCase.terms, testCase.costs, got, testCase.want)
		}
	}
}

// TestRelatedRecallDropsBroadTerms keeps a common title word from expanding rare-term recall.
func TestRelatedRecallDropsBroadTerms(t *testing.T) {
	_, database := queryFixture(t)
	if _, err := database.Exec(`
WITH RECURSIVE documents(id) AS (
 VALUES(1) UNION ALL SELECT id+1 FROM documents WHERE id<40000
)
INSERT INTO article_search(rowid,title,abstract_text)
 SELECT 100000+id,'common 数 字','' FROM documents;
INSERT INTO article_search(rowid,title,abstract_text) VALUES
 (200001,'unusual common',''),(200002,'unusual','');
`); err != nil {
		t.Fatal(err)
	}
	connection, err := database.Conn(t.Context())
	if err != nil {
		t.Fatal(err)
	}
	cache := &relatedDatabaseCosts{values: map[relatedCostKey]int{}}
	cost, err := relatedTermCost(t.Context(), connection, cache, "common", relatedRecallBudget)
	broaderCost, broaderError := relatedTermCost(t.Context(), connection, cache, "common", relatedHanRecallBudget)
	if broaderError != nil || broaderCost != 30001 {
		t.Fatalf("mixed-title counts must not reuse the Latin count cap: %d, %v", broaderCost, broaderError)
	}
	hanCost, hanError := relatedTermCost(t.Context(), connection, nil, "数字", relatedHanRecallBudget)
	if hanError != nil || hanCost != 30004 {
		t.Fatalf("Han phrase count must stop at 7501 documents: %d, %v", hanCost, hanError)
	}
	if closeError := connection.Close(); closeError != nil {
		t.Error(closeError)
	}
	if err != nil || cost != 3001 {
		t.Fatalf("broad term count must stop past the 3000 budget: %d, %v", cost, err)
	}
	ids, err := relatedArticleIds(t.Context(), database, t.TempDir(), "unusual common", 200001, 30)
	if err != nil || !reflect.DeepEqual(ids, []int64{200002}) {
		t.Fatalf("expected only the rare-term candidate, got %v, %v", ids, err)
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
`); err != nil {
		t.Fatal(err)
	}
	rebuildQueryFixtureSearch(t, database)
	related, err := RelatedArticles(context.Background(), configuration, nil, 3001, 10)
	if err != nil {
		t.Fatal(err)
	}
	ids := make([]int64, len(related))
	for index, article := range related {
		ids[index] = int64(article.ArticleID)
	}
	if !reflect.DeepEqual(ids, []int64{3002, 3003}) {
		t.Fatalf("expected title overlap before abstract mentions, got %v", ids)
	}
}

// rebuildQueryFixtureSearch publishes raw canonical fixture fields through the production tokenizer.
func rebuildQueryFixtureSearch(test testing.TB, database *sql.DB) {
	test.Helper()
	connection, err := database.Conn(context.Background())
	if err != nil {
		test.Fatal(err)
	}
	defer func() {
		if err := connection.Close(); err != nil {
			test.Errorf("connection.Close: %v", err)
		}
	}()
	if err := search.Rebuild(context.Background(), connection); err != nil {
		test.Fatal(err)
	}
}
