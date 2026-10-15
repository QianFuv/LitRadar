package index

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"testing"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/storage/indexschema"
)

// TestArticleIdentityResolutionPreservesAliasPriority verifies exact ownership without fuzzy identity merging.
func TestArticleIdentityResolutionPreservesAliasPriority(t *testing.T) {
	article := domain.ArticleDraft{CatalogId: "fixture", Title: "Résearch — 期刊", Doi: new("10.1234/identity"), Pmid: new("42"), PublicationYear: new(int64(2026)), Volume: new("1"), IssueNumber: new("2"), StartPage: new("3")}
	keys := ArticleIdentityKeys(article)
	if len(keys) != 3 {
		t.Fatal("fixture lost its three identity bases", keys)
	}
	for _, scenario := range []struct {
		name     string
		aliases  map[ArticleIdentityKey]int64
		expected ArticleIdentityResolution
		conflict []int64
	}{
		{"new", nil, ArticleIdentityResolution{int64(identity.Stable(keys[0].Value, "article:v1:doi")), false, keys[0]}, nil},
		{"bibliographic-owner", map[ArticleIdentityKey]int64{keys[2]: 9}, ArticleIdentityResolution{9, true, keys[2]}, nil},
		{"same-owner", map[ArticleIdentityKey]int64{keys[0]: 9, keys[1]: 9, keys[2]: 9}, ArticleIdentityResolution{9, true, keys[0]}, nil},
		{"pmid-priority", map[ArticleIdentityKey]int64{keys[1]: 9, keys[2]: 9}, ArticleIdentityResolution{9, true, keys[1]}, nil},
		{"bridge", map[ArticleIdentityKey]int64{keys[0]: 9, keys[1]: 3, keys[2]: 9}, ArticleIdentityResolution{}, []int64{3, 9}},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			actual, err := ResolveArticleIdentity(article, scenario.aliases)
			before := append([]ArticleIdentityKey(nil), keys...)
			reused, reusedError := resolveArticleIdentity(keys, scenario.aliases)
			if !reflect.DeepEqual(keys, before) || reused != actual || (reusedError == nil) != (err == nil) {
				t.Fatal("reused keys changed identity resolution or input ownership", reused, actual, reusedError, err)
			}
			if scenario.conflict != nil {
				var conflict *IdentityConflict
				if !errors.As(err, &conflict) || !reflect.DeepEqual(conflict.ArticleIds, scenario.conflict) {
					t.Fatal("distinct owners were not rejected deterministically", actual, err)
				}
			} else if err != nil || actual != scenario.expected {
				t.Fatal("identity allocation or priority changed", actual, scenario.expected, err)
			}
		})
	}
	if _, err := ResolveArticleIdentity(domain.ArticleDraft{}, nil); err == nil || err.Error() != "article has no canonical identity basis" {
		t.Fatal("missing identity was accepted", err)
	}
}

// BenchmarkContentBatchIdentities includes DOI, PMID and Unicode bibliographic keys in every changed article.
func BenchmarkContentBatchIdentities(b *testing.B) {
	const count = 225
	connection, catalog := projectionBenchmarkFixture(b, count, indexschema.Version)
	batches := []domain.ProviderBatch{projectionUpdateBatch(count, "1"), projectionUpdateBatch(count, "2")}
	for _, batch := range batches {
		for index := range batch.Articles {
			article := &batch.Articles[index]
			article.Title = fmt.Sprintf("Résearch économie — 期刊 article %d", index)
			article.Pmid = new(fmt.Sprint(100000 + index))
			article.PublicationYear, article.Volume, article.IssueNumber = new(int64(2026)), new("1"), new("2")
		}
	}
	if _, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batches[1], "updated", "epoch"); err != nil {
		b.Fatal(err)
	}
	iteration := 0
	b.ReportAllocs()
	for b.Loop() {
		outcome, err := WriteContentBatch(context.Background(), connection.Conn, catalog, batches[iteration%2], "updated", "epoch")
		if err != nil || outcome.ArticlesChanged != count {
			b.Fatal("identity benchmark stopped publishing changes", outcome, err)
		}
		iteration++
	}
}
