package sources

import "testing"

func TestCanonicalIssueOwnsItsObservedFields(t *testing.T) {
	article := CanonicalArticle(ArticleDraft{CatalogId: "journal", Title: "Title", PublicationYear: new(int64(2026)), Volume: new("1"), Doi: new("10.1234/test")})
	if article == nil {
		t.Fatal("identified article lost")
	}
	issue := IssueFromArticle(*article)
	*article.PublicationYear = 2025
	*article.Volume = "2"
	if *issue.PublicationYear != 2026 || *issue.Volume != "1" {
		t.Fatal("issue shares mutable article metadata")
	}
	batch := BatchFromArticles(JournalCatalogEntry{CatalogId: "journal", Title: "Journal"}, []ArticleDraft{*article, *article}, ProviderProgress{State: Complete})
	if len(batch.Articles) != 2 || len(batch.Issues) != 1 {
		t.Fatal("issue deduplication changed article multiplicity")
	}
}
