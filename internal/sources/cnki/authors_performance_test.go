package cnki

import (
	"reflect"
	"strings"
	"testing"
)

// articleAuthorCases covers heading priority, fallback and HTML parsing edge cases.
func articleAuthorCases() []struct{ name, markup, expected string } {
	return []struct{ name, markup, expected string }{
		{"heading", `<h3 class="author" id="authorpart"><span>Alice<sup>1</sup></span><span>Bob</span></h3><span class="author" title="Fallback">Other</span>`, "Alice; Bob"},
		{"title-fallback", `<span class="author" title=" A &amp; B ">Other</span>`, "A & B"},
		{"text-fallback", `<span class="author">Alice <b>Smith</b><sup>1</sup></span>`, "Alice Smith"},
		{"empty-heading", `<h3 class="author" id="authorpart"><span> </span></h3><h3 class="author" id="authorpart"><span>Ignored</span></h3><span class="author" title="Fallback">Other</span>`, "Fallback"},
		{"nested", `<h3 class="author" id="authorpart"><span>Alpha<span>Nested</span><sup>Ignored</sup><script>Ignored</script><style>Ignored</style><template>Ignored</template></span><span>Beta</span></h3>`, "AlphaNested; Beta"},
		{"foster-parenting", `<table><span class="author" title="Earlier">A</span><h3 class="author" id="authorpart"><span>B</span></h3></table>`, "B"},
		{"empty-span", `<span class="author" title=" "> </span><span class="author" title="Next">Other</span>`, "Next"},
		{"no-authors", `<h3>Other heading</h3><span class="unrelated">Other</span>`, ""},
	}
}

// articleDetailFixture includes ordinary page content around the author markup.
func articleDetailFixture(markup string) string {
	return `<input id="paramfilename" value="synthetic"><h1 class="title">Research title</h1><input id="abstract_text" value="Research abstract">` + markup + strings.Repeat(`<div class="content"><span class="label">研究内容 Research content</span><p>Some ordinary article page text.</p></div>`, 200)
}

// TestArticleDetailAuthorPrecedence preserves the existing first-heading and span fallback rules.
func TestArticleDetailAuthorPrecedence(t *testing.T) {
	for _, scenario := range articleAuthorCases() {
		t.Run(scenario.name, func(t *testing.T) {
			value, err := ParseArticleDetail(articleDetailFixture(scenario.markup), KnsBase+"/kcms2/article/abstract?v=synthetic")
			if err != nil {
				t.Fatal(err)
			}
			var expected any
			if scenario.expected != "" {
				expected = scenario.expected
			}
			if actual := value.(map[string]any)["authors"]; !reflect.DeepEqual(actual, expected) {
				t.Fatal("author precedence or text changed", actual, expected)
			}
		})
	}
}

// FuzzArticleDetailAuthors compares shared nodes with the original independently parsed fallback operands.
func FuzzArticleDetailAuthors(f *testing.F) {
	for _, scenario := range articleAuthorCases() {
		f.Add(scenario.markup)
	}
	for _, text := range []string{
		`<table><span class=author>A</span><span class=author>B</span></table>`,
		`<script><span class="author">ignored</span></script><span class="author">Kept</span>`,
		`<svg><span class="author" title="Foreign">A</span></svg><span class="author">B</span>`,
		`<span data-litradar-source-order="1" class="author">A</span><span class="author">B</span>`,
		"<span class=author>\xff</span><span class=author>\x00</span>",
	} {
		f.Add(text)
	}
	f.Fuzz(func(t *testing.T, text string) {
		if len(text) > 4096 {
			t.Skip()
		}
		expected := firstValue(authorText(authorNodes(text)), authorSpanTitle(authorNodes(text)))
		actual := articleDetailAuthors(text)
		if !reflect.DeepEqual(actual, expected) {
			t.Fatal("shared DOM changed author extraction", actual, expected)
		}
	})
}

// BenchmarkParseArticleDetail measures complete detail parsing with and without author metadata.
func BenchmarkParseArticleDetail(b *testing.B) {
	for _, scenario := range articleAuthorCases() {
		if scenario.name != "heading" && scenario.name != "title-fallback" && scenario.name != "no-authors" {
			continue
		}
		b.Run(scenario.name, func(b *testing.B) {
			text := articleDetailFixture(scenario.markup)
			b.ReportAllocs()
			b.SetBytes(int64(len(text)))
			for b.Loop() {
				value, err := ParseArticleDetail(text, KnsBase+"/kcms2/article/abstract?v=synthetic")
				if err != nil || value == nil {
					b.Fatal(value, err)
				}
			}
		})
	}
}
