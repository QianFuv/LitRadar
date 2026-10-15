package query

import (
	"context"
	"testing"

	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// BenchmarkNormalizeArticleParams isolates repeated search-schema work on a warm catalog.
func BenchmarkNormalizeArticleParams(b *testing.B) {
	_, filename := benchmarkCatalog(b)
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		b.Fatal(err)
	}
	defer database.Close()
	if err := database.Ping(); err != nil {
		b.Fatal(err)
	}
	ctx := context.Background()
	params := DefaultArticleListParams()
	b.ReportAllocs()
	for b.Loop() {
		if _, err := normalizeArticleParams(ctx, database, params); err != nil {
			b.Fatal(err)
		}
	}
}
