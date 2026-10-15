package query

import (
	"context"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// BenchmarkIdentifierWorkloads compares ordered scans with identifier materialization across selectivities.
func BenchmarkIdentifierWorkloads(b *testing.B) {
	_, filename := benchmarkCatalog(b)
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		b.Fatal(err)
	}
	defer database.Close()
	if _, err := database.Exec("UPDATE article_listing SET doi='dense' WHERE article_id%2=0"); err != nil {
		b.Fatal(err)
	}
	ctx := context.Background()
	for _, scenario := range []struct{ name, value, scope string }{
		{"Unique", "10.0000/synthetic.30001", ""},
		{"Missing", "missing", ""},
		{"Dense", "dense", ""},
		{"ScopedMissing", "dense", " AND l.issue_id=999999"},
	} {
		for _, strategy := range []struct{ name, condition string }{
			{"Ordered", "l.doi=?"},
			{"Membership", "l.article_id IN (SELECT article_id FROM article_listing WHERE doi=?)"},
		} {
			b.Run(scenario.name+"/"+strategy.name, func(b *testing.B) {
				source := "article_listing l"
				if strategy.name == "Ordered" && scenario.scope != "" {
					issue := int64(999999)
					source = articleListingSource(ArticleListParams{IssueId: &issue, Doi: &scenario.value})
				}
				statement := "SELECT l.article_id FROM " + source + " WHERE " + strategy.condition + scenario.scope + " ORDER BY l.date DESC,l.article_id DESC LIMIT 51"
				b.ReportAllocs()
				for b.Loop() {
					rows, err := database.QueryContext(ctx, statement, scenario.value)
					if err != nil {
						b.Fatal(err)
					}
					for rows.Next() {
						var id int64
						if err := rows.Scan(&id); err != nil {
							b.Fatal(err)
						}
					}
					if err := rows.Err(); err != nil {
						b.Fatal(err)
					}
					rows.Close()
				}
			})
		}
	}
}
