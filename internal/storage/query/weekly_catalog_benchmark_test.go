package query

import (
	"context"
	"encoding/csv"
	"os"
	"path/filepath"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// weeklyCatalogFixture uses maintained journal titles with equal or uneven publication counts.
func weeklyCatalogFixture(b *testing.B, isUneven bool) (config.Config, weekly.Timestamp, int, int) {
	b.Helper()
	file, err := os.Open(filepath.Join("..", "..", "..", "assets", "meta", "english_journals.csv"))
	if err != nil {
		b.Fatal(err)
	}
	records, err := csv.NewReader(file).ReadAll()
	closeError := file.Close()
	if err != nil || closeError != nil {
		b.Fatal(err, closeError)
	}
	titleColumn := -1
	for index, field := range records[0] {
		if field == "title" {
			titleColumn = index
		}
	}
	if titleColumn < 0 || len(records) < 9 {
		b.Fatal("maintained journal catalog is incomplete")
	}
	count := 0
	for index := range records[1:] {
		count++
		if isUneven {
			count += index % 5
		}
	}
	configuration, filename := benchmarkCatalogRows(b, count)
	database, err := storage.OpenPlain(filename)
	if err != nil {
		b.Fatal(err)
	}
	defer database.Close()
	tx, err := database.BeginTx(context.Background(), nil)
	if err != nil {
		b.Fatal(err)
	}
	defer tx.Rollback()
	articleId := 0
	for index, record := range records[1:] {
		journalId := index + 1
		if journalId <= 8 {
			_, err = tx.Exec("UPDATE journals SET title=? WHERE journal_id=?", record[titleColumn], journalId)
		} else {
			_, err = tx.Exec("INSERT INTO journals(journal_id,catalog_id,title,title_aliases_json,issns_json) VALUES(?,?,?,'[]','[]')", journalId, record[0], record[titleColumn])
		}
		if err != nil {
			b.Fatal(err)
		}
		journalCount := 1
		if isUneven {
			journalCount += index % 5
		}
		for range journalCount {
			articleId++
			for _, table := range []string{"articles", "article_listing"} {
				if _, err := tx.Exec("UPDATE "+table+" SET journal_id=?,issue_id=NULL WHERE article_id=?", journalId, articleId); err != nil {
					b.Fatal(err)
				}
			}
		}
	}
	if err := tx.Commit(); err != nil {
		b.Fatal(err)
	}
	ids := make([]int64, count)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	writeQueryManifest(b, configuration, filepath.Base(filename), ids)
	end, _ := weekly.ParseTimestamp("2026-10-04T00:00:00Z")
	return configuration, end, len(records) - 1, count
}

// BenchmarkWeeklyCatalogSummary measures complete warm summaries with maintained catalog titles.
func BenchmarkWeeklyCatalogSummary(b *testing.B) {
	for _, distribution := range []string{"equal", "uneven"} {
		b.Run(distribution, func(b *testing.B) {
			configuration, end, journals, count := weeklyCatalogFixture(b, distribution == "uneven")
			cache := &weekly.Cache{}
			if _, err := WeeklySummary(context.Background(), configuration, end, cache); err != nil {
				b.Fatal(err)
			}
			b.ReportAllocs()
			for b.Loop() {
				result, err := WeeklySummary(context.Background(), configuration, end, cache)
				if err != nil || len(result.Databases) != 1 || len(result.Databases[0].Journals) != journals || result.Databases[0].NewArticleCount != count {
					b.Fatal("maintained-catalog summary changed", result, err)
				}
			}
		})
	}
}
