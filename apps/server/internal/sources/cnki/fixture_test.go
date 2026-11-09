package cnki

import (
	"context"
	"strings"
	"testing"
)

func TestArticleSummaryOwnsIssueMetadata(t *testing.T) {
	issue := map[string]any{"year": map[string]any{"value": "before"}, "number": []any{"one"}}
	text := `<dd class="row clearfix"><a href="/kcms2/article/abstract?v=x">Title</a></dd><input id="articleCount" value="1">`
	page, err := ParseIssueArticles(text, issue, 0)
	if err != nil {
		t.Fatal(err)
	}
	issue["year"].(map[string]any)["value"] = "after"
	issue["number"].([]any)[0] = "two"
	article := page.Articles[0].(map[string]any)
	if article["year"].(map[string]any)["value"] != "before" || article["number"].([]any)[0] != "one" {
		t.Fatal("parsed summary retained caller-owned mutable metadata")
	}
}

func TestFixtureCloneAndAttemptsAreIndependent(t *testing.T) {
	endpoint := "article_detail"
	data := FixtureData{FailEndpoint: &endpoint, ArticleDetailStatusCodes: map[string]uint16{"id": 429}, IssueArticlePages: map[string][]string{"key": {"first"}}}
	fixture := NewFixtureTransport(data)
	data.ArticleDetailStatusCodes["id"] = 200
	data.IssueArticlePages["key"][0] = "changed"
	endpoint = "other"
	id := "id"
	_, err := fixture.ArticleDetail(context.Background(), KnsBase+"/a", &id)
	if err == nil || !strings.HasSuffix(err.Error(), "429") {
		t.Fatal(err)
	}
	clone := fixture.Clone()
	attempts := fixture.Attempts()
	*attempts[0].StatusCode = 201
	*attempts[0].Error = "changed"
	fixture.DrainAttempts()
	if len(clone.Attempts()) != 1 || *clone.Attempts()[0].StatusCode != 429 || *clone.Attempts()[0].Error != "HTTP status" {
		t.Fatal("attempts shared mutable state")
	}
	if fixture.data.IssueArticlePages["key"][0] != "first" || *fixture.data.FailEndpoint != "article_detail" {
		t.Fatal("fixture retained caller state")
	}
}
