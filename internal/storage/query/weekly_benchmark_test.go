package query

import (
	"context"
	"fmt"
	"path/filepath"
	"testing"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/weekly"
)

// weeklyQueryFixture includes canonical bodies, listing membership and a real publication file.
func weeklyQueryFixture(test testing.TB, count int) (config.Config, string, weekly.Timestamp) {
	test.Helper()
	configuration, filename := benchmarkCatalogRows(test, count)
	ids := make([]int64, count, count+2)
	for index := range ids {
		ids[index] = int64(index + 1)
	}
	ids = append(ids, 1, int64(count+1))
	name := filepath.Base(filename)
	writeQueryManifest(test, configuration, name, ids)
	end, _ := weekly.ParseTimestamp("2026-10-04T00:00:00Z")
	return configuration, name, end
}

// BenchmarkWeeklySummary measures file admission, TEMP membership and native grouped summaries.
func BenchmarkWeeklySummary(b *testing.B) {
	for _, count := range []int{1, 120, 10000, 100000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			configuration, _, end := weeklyQueryFixture(b, count)
			for _, isCached := range []bool{false, true} {
				b.Run(fmt.Sprintf("cached=%t", isCached), func(b *testing.B) {
					var cache *weekly.Cache
					if isCached {
						cache = &weekly.Cache{}
						if _, err := WeeklySummary(context.Background(), configuration, end, cache); err != nil {
							b.Fatal(err)
						}
					}
					b.ReportAllocs()
					for b.Loop() {
						result, err := WeeklySummary(context.Background(), configuration, end, cache)
						if err != nil || len(result.Databases) != 1 || result.Databases[0].NewArticleCount != count {
							b.Fatal("weekly summary membership changed", result, err)
						}
					}
				})
			}
		})
	}
}

// BenchmarkWeeklyArticles measures fixed-window pagination including canonical enrichment with a warm manifest.
func BenchmarkWeeklyArticles(b *testing.B) {
	for _, count := range []int{120, 10000, 100000} {
		b.Run(fmt.Sprint(count), func(b *testing.B) {
			configuration, name, end := weeklyQueryFixture(b, count)
			cache := &weekly.Cache{}
			params := WeeklyArticlePageParams{DbName: name, JournalId: 1, WindowEnd: end.Format(true), Limit: 50}
			expected := min((count+7)/8, 50)
			if _, err := WeeklyArticles(context.Background(), configuration, params, cache); err != nil {
				b.Fatal(err)
			}
			b.ReportAllocs()
			for b.Loop() {
				result, err := WeeklyArticles(context.Background(), configuration, params, cache)
				if err != nil || len(result.Items) != expected || result.Page.HasMore == nil || *result.Page.HasMore != (count > 400) {
					b.Fatal("weekly article pagination changed", result, err)
				}
			}
		})
	}
}
