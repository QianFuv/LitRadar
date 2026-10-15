package recommend

import (
	"fmt"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/delivery"
	"github.com/QianFuv/LitRadar/internal/domain/storage"
)

// TestMarkdownExactByteBudget covers inclusive limits, multibyte abstracts, and the tenth count digit.
func TestMarkdownExactByteBudget(t *testing.T) {
	for _, count := range []int{1, 9, 10} {
		t.Run(fmt.Sprint(count), func(t *testing.T) {
			base := []string{"## Weekly Digest for Fixture", "", "- Database: `db`", "- Run ID: `run`"}
			candidates := map[int64]storage.ArticleCandidate{}
			selections := []domain.RankedSelection{}
			sections := []string{}
			for index := range count {
				id := int64(index + 1)
				candidate := storage.ArticleCandidate{ArticleId: id, Title: "Topic", Abstract: "中文"}
				candidates[id] = candidate
				selections = append(selections, domain.RankedSelection{ArticleId: id})
				sections = append(sections, markdownArticleSection(candidate, index+1))
			}
			last := candidates[int64(count)]
			last.Abstract += strings.Repeat("x", MaxPushContentLength-len(renderContent(base, sections)))
			candidates[last.ArticleId] = last
			sections[count-1] = markdownArticleSection(last, count)
			actual := BuildMarkdownContent("db", "run", domain.Subscriber{Name: "Fixture"}, "", selections, candidates)
			if len(actual) != MaxPushContentLength || actual != renderContent(base, sections) {
				t.Fatal("exact byte budget rejected a complete section", len(actual))
			}
			last.Abstract += "x"
			candidates[last.ArticleId] = last
			actual = BuildMarkdownContent("db", "run", domain.Subscriber{Name: "Fixture"}, "", selections, candidates)
			if actual != renderContent(base, sections[:count-1]) {
				t.Fatal("over-budget section was truncated or included", len(actual))
			}
		})
	}
}

// legacyMarkdownBudget retains the previous full-render admission algorithm as a differential oracle.
func legacyMarkdownBudget(base, sections []string) string {
	kept := []string{}
	for _, section := range sections {
		trial := append(append([]string{}, kept...), section)
		if len(renderContent(base, trial)) <= MaxPushContentLength {
			kept = trial
		}
	}
	content := renderContent(base, kept)
	if len(content) <= MaxPushContentLength {
		return content
	}
	characters := []rune(renderContent(base, nil))
	if len(characters) > MaxPushContentLength {
		characters = characters[:MaxPushContentLength]
	}
	return string(characters)
}

// FuzzMarkdownBudget compares the previous renderer with exact accounting over arbitrary literal fields.
func FuzzMarkdownBudget(f *testing.F) {
	f.Add("Reader", " Summary ", "Title", " Abstract ", uint8(20))
	f.Add("中文", "\n\t", "\xff\x00", "中文\r\n", uint8(19))
	f.Add("", "", "", "", uint8(0))
	f.Fuzz(func(t *testing.T, name, summary, title, abstract string, repeats uint8) {
		if len(name)+len(summary)+len(title)+len(abstract) > 2048 {
			t.Skip()
		}
		copies := int(repeats)%20 + 1
		subscriber := domain.Subscriber{Name: strings.Repeat(name, copies)}
		base := []string{"## Weekly Digest for " + subscriber.Name, "", "- Database: `db`", "- Run ID: `run`"}
		if trimmed := strings.TrimSpace(summary); trimmed != "" {
			base = append(base, "", trimmed)
		}
		candidates := map[int64]storage.ArticleCandidate{}
		selections := []domain.RankedSelection{}
		sections := []string{}
		for index := range MaxArticlesPerPush {
			id := int64(index + 1)
			candidate := storage.ArticleCandidate{ArticleId: id, Title: title, Abstract: strings.Repeat(abstract, copies+index)}
			candidates[id] = candidate
			selections = append(selections, domain.RankedSelection{ArticleId: id})
			sections = append(sections, markdownArticleSection(candidate, index+1))
		}
		actual := BuildMarkdownContent("db", "run", subscriber, summary, selections, candidates)
		if expected := legacyMarkdownBudget(base, sections); actual != expected {
			t.Fatalf("markdown byte budget changed: got %q, want %q", actual, expected)
		}
	})
}

// BenchmarkMarkdownContent measures complete twenty-article messages and whole-section overflow.
func BenchmarkMarkdownContent(b *testing.B) {
	for _, mode := range []string{"short", "near-limit", "oversized-first", "unicode", "empty"} {
		b.Run(mode, func(b *testing.B) {
			candidates := map[int64]storage.ArticleCandidate{}
			selections := []domain.RankedSelection{}
			if mode != "empty" {
				for index := range MaxArticlesPerPush {
					id := int64(index + 1)
					abstract := strings.Repeat("Research ", 20)
					if mode == "near-limit" {
						abstract = strings.Repeat("abstract ", 170)
					} else if mode == "unicode" {
						abstract = strings.Repeat("中文", 250)
					} else if mode == "oversized-first" && index == 0 {
						abstract = strings.Repeat("x", MaxPushContentLength)
					}
					candidates[id] = storage.ArticleCandidate{ArticleId: id, Title: fmt.Sprintf("Article %d", id), Abstract: abstract}
					selections = append(selections, domain.RankedSelection{ArticleId: id})
				}
			}
			var content string
			b.ReportAllocs()
			for b.Loop() {
				content = BuildMarkdownContent("db", "run", domain.Subscriber{Name: "Reader"}, "Summary", selections, candidates)
			}
			b.StopTimer()
			if len(content) > MaxPushContentLength || !strings.Contains(content, "Selected Articles:") {
				b.Fatal("invalid message budget", len(content))
			}
		})
	}
}
