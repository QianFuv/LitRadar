package query

import (
	"context"
	"fmt"
	"strings"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// TestCandidateFlattenedValues preserves nullable defaults, nonzero flags and strict SQLite storage classes.
func TestCandidateFlattenedValues(t *testing.T) {
	filename := candidateFixture(t)
	database, err := storage.OpenPlain(filename)
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	for _, scenario := range []struct {
		name, values, abstract, invalidColumn string
		isOpenAccess, isInPress               bool
	}{
		{name: "null", values: "NULL,NULL,NULL"},
		{name: "empty", values: "'',0,0"},
		{name: "nonzero", values: "'abstract',2,-1", abstract: "abstract", isOpenAccess: true, isInPress: true},
		{name: "unicode", values: "'研究',-5,0", abstract: "研究", isOpenAccess: true},
		{name: "abstract-blob", values: "X'61',0,0", invalidColumn: "abstract_text"},
		{name: "abstract-utf8", values: "CAST(X'ff' AS TEXT),0,0", invalidColumn: "abstract_text"},
		{name: "access-text", values: "'abstract','false',0", invalidColumn: "open_access"},
		{name: "access-real", values: "'abstract',1.5,0", invalidColumn: "open_access"},
		{name: "access-blob", values: "'abstract',X'31',0", invalidColumn: "open_access"},
		{name: "inpress-text", values: "'abstract',0,'false'", invalidColumn: "in_press"},
		{name: "inpress-real", values: "'abstract',0,1.5", invalidColumn: "in_press"},
		{name: "inpress-blob", values: "'abstract',0,X'31'", invalidColumn: "in_press"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			if _, err := database.ExecContext(context.Background(), "UPDATE articles SET (abstract_text,open_access,in_press)=("+scenario.values+") WHERE article_id=1"); err != nil {
				t.Fatal(err)
			}
			for _, count := range []int{1, 501} {
				t.Run(fmt.Sprint(count), func(t *testing.T) {
					ids := make([]int64, count)
					ids[0] = 1
					for index := 1; index < count; index++ {
						ids[index] = int64(100000 + index)
					}
					items, err := FetchCandidatesForArticleIds(context.Background(), filename, ids)
					if scenario.invalidColumn != "" {
						if err == nil || items != nil || !strings.Contains(err.Error(), `name "`+scenario.invalidColumn+`"`) || !strings.Contains(err.Error(), "invalid SQLite") {
							t.Fatal("invalid stored value was accepted or changed diagnostics", items, err)
						}
						return
					}
					if err != nil || len(items) != 1 {
						t.Fatal("candidate membership changed", items, err)
					}
					item := items[0]
					if item.ArticleId != 1 || item.Abstract != scenario.abstract || item.OpenAccess != scenario.isOpenAccess || item.InPress != scenario.isInPress || item.IssueId == nil || *item.IssueId != 7 || item.Date == nil || *item.Date != "2026-10-01" || item.Doi != nil {
						t.Fatal("flattened or retained nullable fields changed", item)
					}
				})
			}
		})
	}
}
