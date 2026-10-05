package query

import (
	"context"
	"fmt"
	"path/filepath"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	migration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

func benchmarkCatalog(benchmark testing.TB) (config.Config, string) {
	benchmark.Helper()
	configuration := config.FromProjectRoot(benchmark.TempDir())
	filename := filepath.Join(configuration.IndexDir, "benchmark.sqlite")
	ctx := context.Background()
	if _, err := migration.Migrate(ctx, filename); err != nil {
		benchmark.Fatal(err)
	}
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		benchmark.Fatal(err)
	}
	defer database.Close()
	_, err = database.ExecContext(ctx, `BEGIN;
WITH RECURSIVE sequence(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM sequence WHERE id<8)
INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json)
SELECT id,printf('synthetic-%d',id),printf('Synthetic Journal %d',id),'[]','[]' FROM sequence;
WITH RECURSIVE sequence(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM sequence WHERE id<32)
INSERT INTO issues(issue_id,journal_id,publication_year,title,volume,number)
SELECT id,(id-1)%8+1,2026,'Synthetic Issue','1',printf('%d',id) FROM sequence;
WITH RECURSIVE sequence(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM sequence WHERE id<60000)
INSERT INTO articles(article_id,journal_id,issue_id,title,publication_year,date,authors_json,abstract_text,doi,pmid,open_access,in_press)
SELECT id,(id-1)%8+1,CASE WHEN id%4=0 THEN NULL ELSE (id-1)%32+1 END,
'Synthetic Article',2026,CASE WHEN id%17=0 THEN NULL ELSE printf('2026-09-%02d',id%28+1) END,
'["Synthetic Author"]','Synthetic abstract for repeatable benchmark measurements.',
printf('10.0000/synthetic.%d',id),printf('%d',id),id%2,id%4=0 FROM sequence;
INSERT INTO article_listing(article_id,journal_id,issue_id,publication_year,date,open_access,in_press,doi,pmid)
SELECT article_id,journal_id,issue_id,publication_year,date,open_access,in_press,doi,pmid FROM articles;
INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title)
SELECT a.article_id,a.article_id,a.title,a.abstract_text,a.doi,a.pmid,'Synthetic Author',j.title
FROM articles a JOIN journals j ON j.journal_id=a.journal_id;
COMMIT;`)
	if err != nil {
		benchmark.Fatal(err)
	}
	return configuration, filename
}

func BenchmarkCandidates(b *testing.B) {
	_, filename := benchmarkCatalog(b)
	for _, count := range []int{120, 500, 4096, 32766} {
		b.Run(fmt.Sprintf("Ids%d", count), func(b *testing.B) {
			ids := make([]int64, count)
			for index := range ids {
				ids[index] = int64(index + 1)
			}
			b.ReportAllocs()
			for b.Loop() {
				items, err := FetchCandidatesForArticleIds(context.Background(), filename, ids)
				if err != nil || len(items) != count {
					b.Fatalf("items=%d error=%v", len(items), err)
				}
			}
		})
	}
	for _, count := range []int{32767, 65536} {
		ids := make([]int64, count)
		for index := range ids {
			ids[index] = int64(index + 1)
		}
		items, err := FetchCandidatesForArticleIds(context.Background(), filename, ids)
		b.Logf("untimed limit probe: ids=%d results=%d error=%v", count, len(items), err)
	}
}

func BenchmarkArticleLookup(b *testing.B) {
	configuration, filename := benchmarkCatalog(b)
	ctx := context.Background()
	for _, field := range []string{"doi", "pmid"} {
		for _, hasMatch := range []bool{true, false} {
			for _, includesTotal := range []bool{true, false} {
				b.Run(fmt.Sprintf("%s/hit=%t/total=%t", field, hasMatch, includesTotal), func(b *testing.B) {
					params := DefaultArticleListParams()
					value := "30001"
					if !hasMatch {
						value = "999999"
					}
					if field == "doi" {
						value = "10.0000/synthetic." + value
						params.Doi = &value
					} else {
						params.Pmid = &value
					}
					params.IncludeTotal = &includesTotal
					probe, err := ListArticles(ctx, configuration, nil, params)
					if err != nil || len(probe.Items) > 1 || (len(probe.Items) == 1) != hasMatch {
						b.Fatalf("invalid lookup fixture: items=%d error=%v", len(probe.Items), err)
					}
					if hasMatch && int64(probe.Items[0].ArticleId) != 30001 {
						b.Fatal("lookup returned a different article")
					}
					if includesTotal {
						if probe.Page.Total == nil || *probe.Page.Total != int64(len(probe.Items)) {
							b.Fatal("lookup omitted or miscounted total")
						}
					} else if probe.Page.Total != nil {
						b.Fatal("lookup unexpectedly counted total")
					}
					b.ReportAllocs()
					for b.Loop() {
						page, err := ListArticles(ctx, configuration, nil, params)
						if err != nil || len(page.Items) > 1 || (len(page.Items) == 1) != hasMatch {
							b.Fatalf("items=%d error=%v", len(page.Items), err)
						}
					}
				})
			}
		}
	}
	b.Run("OpenSimple", func(b *testing.B) {
		b.ReportAllocs()
		for b.Loop() {
			database, err := storage.Open(filename, false, 1)
			if err != nil {
				b.Fatal(err)
			}
			var value int
			err = database.QueryRowContext(ctx, "SELECT 1").Scan(&value)
			closeError := database.Close()
			if err != nil || closeError != nil || value != 1 {
				b.Fatal(err, closeError, value)
			}
		}
	})
	b.Run("WarmSelect", func(b *testing.B) {
		database, err := storage.Open(filename, false, 1)
		if err != nil {
			b.Fatal(err)
		}
		defer database.Close()
		if err := database.PingContext(ctx); err != nil {
			b.Fatal(err)
		}
		b.ReportAllocs()
		for b.Loop() {
			var value int
			if err := database.QueryRowContext(ctx, "SELECT 1").Scan(&value); err != nil || value != 1 {
				b.Fatal(err, value)
			}
		}
	})
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		b.Fatal(err)
	}
	defer database.Close()
	for _, statement := range []string{
		"SELECT COUNT(*) FROM article_listing l WHERE l.doi='10.0000/synthetic.30001'",
		"SELECT l.article_id,l.date FROM article_listing l WHERE l.doi='10.0000/synthetic.30001' ORDER BY l.date DESC,l.article_id DESC LIMIT 51 OFFSET 0",
		"SELECT article_id FROM articles WHERE doi='10.0000/synthetic.30001'",
		"SELECT COUNT(*) FROM article_listing l WHERE l.pmid='30001'",
		"SELECT l.article_id,l.date FROM article_listing l WHERE l.pmid='30001' ORDER BY l.date DESC,l.article_id DESC LIMIT 51 OFFSET 0",
		"SELECT article_id FROM articles WHERE pmid='30001'",
	} {
		rows, err := database.QueryContext(ctx, "EXPLAIN QUERY PLAN "+statement)
		if err != nil {
			b.Fatal(err)
		}
		for rows.Next() {
			var id, parent, unused int
			var detail string
			if err := rows.Scan(&id, &parent, &unused, &detail); err != nil {
				b.Fatal(err)
			}
			b.Logf("query plan: %s => %s", statement, detail)
		}
		if err := rows.Err(); err != nil {
			b.Fatal(err)
		}
		rows.Close()
	}
}
