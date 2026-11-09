package scholarly

import (
	"context"
	"encoding/json"
	"math"
	"testing"
)

func TestCrossrefBoundsPrecedeTimestampRange(t *testing.T) {
	_, err := CrossrefParameters(CrossrefQuery{CreatedFrom: 0, CreatedUntil: math.MinInt64})
	if err == nil || err.Error() != "invalid Crossref date bounds" {
		t.Fatalf("wrong validation priority: %v", err)
	}
}
func TestRedactKeyWithoutEquals(t *testing.T) {
	if actual := RedactUrl("https://example.test/?api_key&cursor&x=1"); actual != "https://example.test/?api_key=SECRET&cursor=REDACTED&x=1" {
		t.Fatal(actual)
	}
}
func TestFixtureJsonPreservesTimestampKind(t *testing.T) {
	var data FixtureData
	if err := json.Unmarshal([]byte(`{"crossref_works":[{"created":{"timestamp":1000}},{"created":{"timestamp":1000.0}}]}`), &data); err != nil {
		t.Fatal(err)
	}
	client := NewClient(NewFixtureTransport(data), false)
	page, err := client.FetchCrossrefPage(context.Background(), "X", CrossrefQuery{CreatedFrom: 1, CreatedUntil: 1})
	if err != nil || page.TotalResults != 1 {
		t.Fatalf("integer timestamp lost: %#v %v", page, err)
	}
}
func TestFixtureRequiredFieldsRejectNull(t *testing.T) {
	for _, key := range []string{"crossref_works", "crossref_work_pages", "openalex_source_works", "openalex_source_work_pages", "openalex_source_works_plan_restricted", "openalex_by_doi", "semantic_scholar_by_doi"} {
		var data FixtureData
		if err := json.Unmarshal([]byte(`{"`+key+`":null}`), &data); err == nil {
			t.Fatalf("accepted null %s", key)
		}
	}
}
func TestProgrammaticFixtureIntegerTimestamp(t *testing.T) {
	for _, value := range []any{int(1000), int64(1000), uint64(1000), json.Number("1000")} {
		if actual := fixtureCreated(map[string]any{"created": map[string]any{"timestamp": value}}); actual != 1 {
			t.Fatalf("%T timestamp became %d", value, actual)
		}
	}
	if fixtureCreated(map[string]any{"created": map[string]any{"timestamp": float64(1000)}}) != 0 {
		t.Fatal("float accepted as integer")
	}
}
