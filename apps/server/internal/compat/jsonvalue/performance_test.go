package jsonvalue

import (
	"strings"
	"testing"
)

// BenchmarkValidJson measures settings and article-batch validation without fixture setup costs.
func BenchmarkValidJson(b *testing.B) {
	for _, scenario := range []struct{ name, value string }{
		{"Settings", `{"enabled":true,"limit":50,"name":"文献追踪"}`},
		{"Articles", `[` + strings.Repeat(`{"title":"文献 [1e400]","year":2026,"doi":"10.1234/example","open":true},`, 999) + `null]`},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			b.ReportAllocs()
			b.SetBytes(int64(len(scenario.value)))
			for b.Loop() {
				if !ValidJson(scenario.value) {
					b.Fatal("valid fixture rejected")
				}
			}
		})
	}
}

// BenchmarkEncodeJson measures common response text and literal separator conversion.
func BenchmarkEncodeJson(b *testing.B) {
	for _, scenario := range []struct{ name, value string }{
		{"Settings", "Enabled tracking"},
		{"ArticleText", strings.Repeat("Research abstract 文献摘要. ", 4096)},
		{"Separators", strings.Repeat("Research\u2028abstract\u2029", 4096)},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			b.ReportAllocs()
			b.SetBytes(int64(len(scenario.value)))
			for b.Loop() {
				if _, err := EncodeJson(scenario.value); err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}
