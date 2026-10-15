package search

import (
	"strings"
	"testing"
	"unicode"

	"golang.org/x/text/unicode/norm"
)

// FuzzPrepareTextMatchesRuneWriter protects normalization against the original rune-writing contract.
func FuzzPrepareTextMatchesRuneWriter(f *testing.F) {
	for _, value := range []string{"ABC\tDEF", "École Ö 中文!", "\u0301\u0345", "\ufffd\xff", "\ue000\U0010fffd", "한글"} {
		f.Add(value)
	}
	f.Fuzz(func(t *testing.T, value string) {
		expected := value
		if !isAsciiProjection(value) {
			var output strings.Builder
			for _, character := range norm.NFD.String(value) {
				if unicode.IsMark(character) {
					continue
				}
				isAlphanumeric := unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character)
				if !isAlphanumeric && !isPrivateProjectionRune(character) {
					output.WriteByte(' ')
				} else if character < 128 {
					output.WriteRune(character)
				} else {
					output.WriteRune(unicode.ToLower(character))
				}
			}
			expected = output.String()
		}
		if actual := PrepareText(value, true); actual != expected {
			t.Fatalf("projection %q: got %q, want %q", value, actual, expected)
		}
	})
}

// BenchmarkPrepareText measures projection work across unchanged and normalized article text.
func BenchmarkPrepareText(b *testing.B) {
	for _, scenario := range []struct{ name, value string }{
		{"Ascii", strings.Repeat("Research abstract ", 4096)},
		{"Punctuation", strings.Repeat("Research: title, methods & results. ", 4096)},
		{"Chinese", strings.Repeat("文献研究与分析", 4096)},
		{"Accents", strings.Repeat("Café naïve résumé ", 4096)},
		{"Marks", strings.Repeat("\u0301\u0308", 4096)},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			b.ReportAllocs()
			b.SetBytes(int64(len(scenario.value)))
			for b.Loop() {
				PrepareText(scenario.value, true)
			}
		})
	}
}
