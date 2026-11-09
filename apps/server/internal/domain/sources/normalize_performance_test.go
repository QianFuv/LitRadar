package sources

import (
	"strings"
	"testing"

	"golang.org/x/text/cases"
	"golang.org/x/text/language"
)

// TestLowercaseRetainsUnicodeMapping covers ASCII controls and context-sensitive full string mappings.
func TestLowercaseRetainsUnicodeMapping(t *testing.T) {
	ascii := make([]byte, 128)
	for index := range ascii {
		ascii[index] = byte(index)
	}
	for _, value := range []string{"", string(ascii), "10.1234/MIXED.DOI", "ΟΣ ΟΣΑ", "İSTANBUL", "I\u0307 Σ\u0301", "\xffASCII\xc0\x80", strings.Repeat("ASCII ", 1000) + "Σ"} {
		if actual, expected := Lowercase(value), cases.Lower(language.Und).String(value); actual != expected {
			t.Fatalf("input %q: got %q, want %q", value, actual, expected)
		}
	}
	if Lowercase("ΟΣ") != "ος" || Lowercase("İ") != "i\u0307" {
		t.Fatal("full Unicode lowercasing was replaced by simple rune mapping")
	}
}

// FuzzLowercase compares optimized normalization with the original Unicode transformer on arbitrary bytes.
func FuzzLowercase(f *testing.F) {
	for _, value := range []string{"", "Title ABSTRACT 123", "lowercase", "ΟΣ ΟΣΑ", "İSTANBUL", "\xffASCII\xc0\x80", "a\x00Z\x7f"} {
		f.Add(value)
	}
	f.Fuzz(func(t *testing.T, value string) {
		if len(value) > 65536 {
			t.Skip()
		}
		if actual, expected := Lowercase(value), cases.Lower(language.Und).String(value); actual != expected {
			t.Fatalf("input %q: got %q, want %q", value, actual, expected)
		}
	})
}

// BenchmarkLowercase compares short identifiers and long English text with Unicode fallback positions.
func BenchmarkLowercase(b *testing.B) {
	for _, scenario := range []struct {
		name  string
		value string
	}{
		{"doi", "10.1234/MixedCase.Article"},
		{"ascii-lower", strings.Repeat("an english research abstract. ", 70)},
		{"ascii-mixed", strings.Repeat("An English RESEARCH Abstract. ", 70)},
		{"unicode-early", "Σ " + strings.Repeat("An English Research Abstract. ", 70)},
		{"unicode-late", strings.Repeat("An English Research Abstract. ", 70) + " Σ"},
	} {
		for _, isReference := range []bool{false, true} {
			name := scenario.name + "/optimized"
			if isReference {
				name = scenario.name + "/reference"
			}
			b.Run(name, func(b *testing.B) {
				b.ReportAllocs()
				for b.Loop() {
					if isReference {
						cases.Lower(language.Und).String(scenario.value)
					} else {
						Lowercase(scenario.value)
					}
				}
			})
		}
	}
}
