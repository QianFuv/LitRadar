package query

import (
	"context"
	"testing"
)

// TestIdentifierFiltersKeepProjectionAuthorityAndPaging protects nonunique identifiers and null dates.
func TestIdentifierFiltersKeepProjectionAuthorityAndPaging(t *testing.T) {
	configuration, database := queryFixture(t)
	if _, err := database.Exec(`UPDATE articles SET doi='canonical-doi',pmid='canonical-pmid';
UPDATE article_listing SET doi='lookup-doi',pmid='lookup-pmid';`); err != nil {
		t.Fatal(err)
	}
	for _, sort := range []string{"date:asc", "date:desc"} {
		for _, field := range []string{"doi", "pmid", "both"} {
			t.Run(sort+"/"+field, func(t *testing.T) {
				params := DefaultArticleListParams()
				params.Sort, params.Limit = &sort, 1
				doi, pmid := " lookup-doi ", " lookup-pmid "
				if field != "pmid" {
					params.Doi = &doi
				}
				if field != "doi" {
					params.Pmid = &pmid
				}
				expected := []int64{1001, 1012}
				if sort == "date:desc" {
					expected = []int64{1012, 1001}
				}
				for index, id := range expected {
					page, err := ListArticles(context.Background(), configuration, nil, params)
					if err != nil || len(page.Items) != 1 || int64(page.Items[0].ArticleId) != id {
						t.Fatalf("page %d: %+v, %v", index, page, err)
					}
					if index == 0 && (page.Page.Total == nil || *page.Page.Total != 2 || page.Page.NextCursor == nil) {
						t.Fatalf("missing duplicate count or continuation: %+v", page.Page)
					}
					params.Cursor = page.Page.NextCursor
				}
			})
		}
	}
	params := DefaultArticleListParams()
	canonical := "canonical-doi"
	params.Doi = &canonical
	page, err := ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 0 {
		t.Fatalf("canonical identity overrode listing projection: %+v, %v", page, err)
	}
	doi, pmid := "lookup-doi", "missing-pmid"
	params.Doi, params.Pmid = &doi, &pmid
	page, err = ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 0 {
		t.Fatalf("identifier filters did not intersect: %+v, %v", page, err)
	}
	issue := int64(999999)
	pmid = "lookup-pmid"
	params.IssueId = &issue
	page, err = ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 0 || page.Page.Total == nil || *page.Page.Total != 0 {
		t.Fatalf("empty issue scope changed: %+v, %v", page, err)
	}
	if _, err := database.Exec(`INSERT INTO issues(issue_id,journal_id) VALUES(999999,1);
UPDATE articles SET issue_id=999999 WHERE article_id=1001;
UPDATE article_listing SET issue_id=999999 WHERE article_id=1001;`); err != nil {
		t.Fatal(err)
	}
	page, err = ListArticles(context.Background(), configuration, nil, params)
	if err != nil || len(page.Items) != 1 || page.Items[0].ArticleId != 1001 || page.Page.Total == nil || *page.Page.Total != 1 {
		t.Fatalf("matching issue scope changed: %+v, %v", page, err)
	}
}
