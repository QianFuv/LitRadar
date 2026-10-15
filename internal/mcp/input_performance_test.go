package mcp

import (
	"encoding/json"
	"testing"
)

// declaredInputSchema resolves the actual embedded declaration used by the registered tool.
func declaredInputSchema(test testing.TB, name string) toolSchema {
	test.Helper()
	var tools []toolDeclaration
	if err := json.Unmarshal(declarations, &tools); err != nil {
		test.Fatal(err)
	}
	for _, tool := range tools {
		if tool.Name == name {
			var schema toolSchema
			if err := json.Unmarshal(tool.InputSchema, &schema); err != nil {
				test.Fatal(err)
			}
			prepareToolSchema(&schema)
			return schema
		}
	}
	test.Fatal("embedded tool was not found", name)
	return toolSchema{}
}

// BenchmarkDecodeInput includes JSON admission and typed conversion for real embedded tool schemas.
func BenchmarkDecodeInput(b *testing.B) {
	for _, scenario := range []struct {
		name, tool, payload, field string
		expected                   any
	}{
		{"article", "get_article", `{"article_id":"42","db":"fixture.sqlite"}`, "article_id", "42"},
		{"search-typical", "search_articles", `{"q":"research","db":"fixture.sqlite","limit":50,"open_access":true}`, "limit", int64(50)},
		{"search-all", "search_articles", `{"abs_rating":["4*"],"area":["Research"],"cursor":"next","date_from":"2026-01-01","date_to":"2026-10-01","db":"fixture.sqlite","doi":"10.1234/research","fms_rating":["A"],"fmscn_rating":["A"],"in_press":false,"include_total":true,"issue_id":3,"journal_id":["1","2"],"limit":50,"offset":0,"open_access":true,"pmid":"42","q":"research","search_mode":"simple","sort":"date:desc","utd_rating":["A"],"year":2026}`, "year", int64(2026)},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			schema := declaredInputSchema(b, scenario.tool)
			raw := json.RawMessage(scenario.payload)
			b.ReportAllocs()
			for b.Loop() {
				input, err := decodeInput(raw, schema)
				if err != nil || input.fields[scenario.field] != scenario.expected {
					b.Fatal("typed input changed", input, err)
				}
			}
		})
	}
}
