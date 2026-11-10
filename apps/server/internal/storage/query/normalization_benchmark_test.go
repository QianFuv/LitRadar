package query

import "testing"

// BenchmarkPreparedMatch isolates query tokenization and phrase construction.
func BenchmarkPreparedMatch(b *testing.B) {
	params := DefaultArticleListParams()
	params.Query = new("研究 Deep-Learning")
	b.ReportAllocs()
	for b.Loop() {
		_ = preparedMatch(params)
	}
}
