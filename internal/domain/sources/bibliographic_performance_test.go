package sources

import (
	"strings"
	"testing"
	"unicode"

	"golang.org/x/text/unicode/norm"
)

// bibliographicTextReference preserves the original scalar mapping and final whitespace collapse.
func bibliographicTextReference(value string) string {
	var output strings.Builder
	for _, character := range norm.NFC.String(value) {
		if character == '\u0130' {
			output.WriteString("i ")
			continue
		}
		character = unicode.ToLower(character)
		if unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) {
			output.WriteRune(character)
		} else {
			output.WriteByte(' ')
		}
	}
	return strings.Join(strings.Fields(output.String()), " ")
}

// TestBibliographicNormalizationPreservesIdentity pins Unicode and numeric distinctions used by durable identity keys.
func TestBibliographicNormalizationPreservesIdentity(t *testing.T) {
	for input, expected := range map[string]string{
		"": "", "---": "", " A__B\u00a0C\u200dD ": "a b c d", "AİB": "ai b", "İİ": "i i", "I\u0307X": "i x",
		"ΟΣ ΟΣΑ": "οσ οσα", " e\u0301—\tCafé ": "é café", "密。\u2003文": "密 文", "\xffA\xc0\x80B": "a b", "ȺȾ": "ⱥⱦ", "１２Ⅳ": "１２ⅳ",
	} {
		if actual := NormalizeBibliographicText(input); actual != expected {
			t.Fatal("bibliographic identity changed", input, actual, expected)
		}
	}
	for input, expected := range map[string]string{"": "0", "000": "0", "00012": "12", "１２": "１２", "000-A": "000 a"} {
		if actual := NormalizeBibliographicLabel(input); actual != expected {
			t.Fatal("bibliographic label changed", input, actual, expected)
		}
	}
}

// FuzzBibliographicNormalization compares durable normalized identity text with its original implementation.
func FuzzBibliographicNormalization(f *testing.F) {
	for _, value := range []string{"", "Article title 12", "AİB", "I\u0307X", "ΟΣ", "e\u0301", "\u0345", "a\u0345", "ȺȾ", "\xffA\xc0\x80B", "１２Ⅳ", "a\u00a0b\u200dc"} {
		f.Add(value)
	}
	f.Fuzz(func(t *testing.T, value string) {
		if len(value) > 8192 {
			t.Skip()
		}
		if actual, expected := NormalizeBibliographicText(value), bibliographicTextReference(value); actual != expected {
			t.Fatalf("identity changed: %q != %q for %q", actual, expected, value)
		}
	})
}

// BenchmarkBibliographicNormalization covers issue labels, article titles and discarded separators.
func BenchmarkBibliographicNormalization(b *testing.B) {
	for _, scenario := range []struct {
		name  string
		value string
	}{
		{"label", "00012"},
		{"english", "Research article about deterministic distributed systems"},
		{"unicode", "Résearch économie — 期刊 article with İ and ΟΣ"},
		{"punctuation", strings.Repeat("-- \t。", 1000)},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				NormalizeBibliographicText(scenario.value)
			}
		})
	}
}
