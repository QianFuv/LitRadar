package recommend

import (
	"fmt"
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/storage"
)

// TestSelectionPreferenceRanking preserves duplicate preferences, model entries, and Unicode matching.
func TestSelectionPreferenceRanking(t *testing.T) {
	subscriber := domain.Subscriber{SubscriberId: "reader", Keywords: []string{" DATA ", "data", "Café", " "}, Directions: []string{"方向"}}
	candidates := map[int64]storage.ArticleCandidate{
		1: {ArticleId: 1, Title: "data"},
		2: {ArticleId: 2, Title: "DATA CAFÉ"},
		3: {ArticleId: 3, Title: "data café 方向"},
		4: {ArticleId: 4, Title: "DATA"},
		5: {ArticleId: 5, Title: "DATA CAFÉ 方向"},
		6: {ArticleId: 6, Title: "unrelated"},
		7: {ArticleId: 7, Title: "CAFÉ"},
	}
	result := domain.SelectionResult{Selections: []domain.RankedSelection{{ArticleId: 1, Score: 10}, {ArticleId: 999, Score: 999}, {ArticleId: 2, Score: 1}, {ArticleId: 3, Score: 999}, {ArticleId: 1, Score: 9}, {ArticleId: 5, Score: -1}, {ArticleId: 6, Score: 100}, {ArticleId: 7, Score: 99999}}}
	dedupe := map[string]string{"reader:3": "delivered"}
	before := append([]domain.RankedSelection(nil), result.Selections...)
	selections := ApplySelectionRules(result, subscriber, candidates, dedupe)
	want := []domain.RankedSelection{result.Selections[5], result.Selections[2], result.Selections[0], result.Selections[4], {ArticleId: 4}, result.Selections[7], result.Selections[6]}
	if !reflect.DeepEqual(selections, want) || !reflect.DeepEqual(result.Selections, before) {
		t.Fatal("preference priority, duplicate identity, or input ownership changed", selections)
	}
	subscriber.Keywords, subscriber.Directions = []string{"unrelated"}, nil
	if changed := ApplySelectionRules(result, subscriber, candidates, dedupe); changed[0].ArticleId != 6 {
		t.Fatal("scores leaked between subscribers", changed)
	}
	candidates[6] = storage.ArticleCandidate{ArticleId: 6, Title: "no longer relevant"}
	if changed := ApplySelectionRules(result, subscriber, candidates, dedupe); changed[0].ArticleId != 7 {
		t.Fatal("scores survived a candidate content change", changed)
	}
}

// TestSelectionScoresUseMapKeys preserves distinct qualification and comparison lookups for malformed maps.
func TestSelectionScoresUseMapKeys(t *testing.T) {
	candidates := map[int64]storage.ArticleCandidate{
		1: {ArticleId: 2, Title: "match"},
		2: {ArticleId: 3, Title: "unrelated"},
		3: {ArticleId: 4, Title: "match"},
	}
	result := domain.SelectionResult{Selections: []domain.RankedSelection{{ArticleId: 1}}}
	actual := ApplySelectionRules(result, domain.Subscriber{Keywords: []string{"match"}}, candidates, nil)
	want := []domain.RankedSelection{{ArticleId: 1}, {ArticleId: 4}, {ArticleId: 2}}
	if !reflect.DeepEqual(actual, want) {
		t.Fatal("candidate field identity replaced map lookup semantics", actual)
	}
}

// TestSelectionScoreTiesRetainModelOrder keeps stable ordering after model-score quantization.
func TestSelectionScoreTiesRetainModelOrder(t *testing.T) {
	result := domain.SelectionResult{}
	candidates := map[int64]storage.ArticleCandidate{}
	for index := range MaxArticlesPerPush {
		id := int64(MaxArticlesPerPush - index)
		candidates[id] = storage.ArticleCandidate{ArticleId: id, Title: "match"}
		result.Selections = append(result.Selections, domain.RankedSelection{ArticleId: id, Score: 1 + float64(index%2)*0.0000001})
	}
	actual := ApplySelectionRules(result, domain.Subscriber{Keywords: []string{"match"}}, candidates, nil)
	if !reflect.DeepEqual(actual, result.Selections) {
		t.Fatal("quantized score ties lost model order", actual)
	}
}

// BenchmarkSelectionRules includes supplemental sorting, already-full model results, and no-match scans.
func BenchmarkSelectionRules(b *testing.B) {
	for _, count := range []int{20, 120, 1000} {
		for _, mode := range []string{"supplement", "full-model", "unmatched"} {
			b.Run(fmt.Sprintf("candidates=%d/%s", count, mode), func(b *testing.B) {
				candidates := make(map[int64]storage.ArticleCandidate, count)
				for index := range count {
					id := int64(index + 1)
					candidates[id] = storage.ArticleCandidate{ArticleId: id, Title: "MATCH CAFÉ 方向", Abstract: strings.Repeat("Unicode Σ research abstract. ", 70)}
				}
				subscriber := domain.Subscriber{Keywords: []string{" match ", "MATCH", "Café"}, Directions: []string{"方向", "σ", "absent"}}
				result := domain.SelectionResult{}
				expected := MaxArticlesPerPush
				if mode == "full-model" {
					for index := range MaxArticlesPerPush {
						result.Selections = append(result.Selections, domain.RankedSelection{ArticleId: int64(index + 1), Score: float64(index % 7)})
					}
				} else if mode == "unmatched" {
					subscriber.Keywords, subscriber.Directions = []string{"not present", "unmatched"}, nil
					expected = 0
				}
				var selections []domain.RankedSelection
				b.ReportAllocs()
				for b.Loop() {
					selections = ApplySelectionRules(result, subscriber, candidates, nil)
				}
				b.StopTimer()
				if len(selections) != expected || mode == "supplement" && selections[0].ArticleId != int64(count) {
					b.Fatal("selection behavior changed", selections)
				}
			})
		}
	}
}

// BenchmarkEnglishSelectionRules measures the usual English article path through preference ranking.
func BenchmarkEnglishSelectionRules(b *testing.B) {
	for _, count := range []int{120, 1000} {
		b.Run(fmt.Sprintf("candidates=%d", count), func(b *testing.B) {
			candidates := make(map[int64]storage.ArticleCandidate, count)
			for index := range count {
				id := int64(index + 1)
				candidates[id] = storage.ArticleCandidate{ArticleId: id, Title: "Research Article", Abstract: strings.Repeat("An English RESEARCH Abstract. ", 70)}
			}
			subscriber := domain.Subscriber{Keywords: []string{" research ", "RESEARCH", "english"}, Directions: []string{"abstract", "article", "absent"}}
			var selected []domain.RankedSelection
			b.ReportAllocs()
			for b.Loop() {
				selected = ApplySelectionRules(domain.SelectionResult{}, subscriber, candidates, nil)
			}
			b.StopTimer()
			if len(selected) != MaxArticlesPerPush || selected[0].ArticleId != int64(count) {
				b.Fatal("English ranking changed", selected)
			}
		})
	}
}
