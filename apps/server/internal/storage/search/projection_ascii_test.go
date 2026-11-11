package search

import (
	"fmt"
	"strings"
	"testing"
)

// TestProjectionAsciiSeparators pins every ASCII byte in fast-path and normalized text.
func TestProjectionAsciiSeparators(t *testing.T) {
	for character := range rune(128) {
		t.Run(fmt.Sprintf("%02x", character), func(t *testing.T) {
			value := string(character)
			expected := " "
			if character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' {
				expected = value
			}
			if actual := FoldText("!" + value + "É"); actual != " "+expected+"E" {
				t.Fatalf("normalized ASCII %02x: got %q", character, actual)
			}
			if strings.ContainsRune(" \t\n\r\f", character) {
				expected = value
			}
			if actual := FoldText(value); actual != expected {
				t.Fatalf("standalone ASCII %02x: got %q, want %q", character, actual, expected)
			}
			projected := ""
			if expected == value && !strings.ContainsRune(" \t\n\r\f", character) {
				projected = strings.ToLower(value)
			}
			if actual := PrepareText(value); actual != projected {
				t.Fatalf("tokenized ASCII %02x: got %q, want %q", character, actual, projected)
			}
		})
	}
}

// TestPrepareTextFoldsBeforeTokenization pins legacy projection semantics separately from raw tokens.
func TestPrepareTextFoldsBeforeTokenization(t *testing.T) {
	for _, scenario := range []struct{ input, folded, prepared string }{
		{"Resume  AB12\tZ", "Resume  AB12\tZ", "resume ab 12 z"},
		{"a-b", "a b", "a b"},
		{"a-bÉ", "a bE", "a be"},
		{"Résumé École cafe\u0301 CAFÉ", "Resume Ecole cafe CAFE", "resume ecole cafe cafe"},
		{"\u0301\u0308Émile", "Emile", "emile"},
		{"ŁÓDŹ Ω Б", "łODZ ω б", "ł odz ω б"},
		{"𠀀😀中文", "𠀀 中文", "𠀀 中 文"},
		{"\ue000\uf8ff\U000f0000\U000ffffd\U00100000\U0010fffd", "\ue000\uf8ff\U000f0000\U000ffffd\U00100000\U0010fffd", "\ue000 \uf8ff \U000f0000 \U000ffffd \U00100000 \U0010fffd"},
		{"\ud7ff\uf900\U000effff\U000ffffe\U0010fffe", " 豈   ", "豈"},
		{"\x80\xc0\xaf\xe1AB", "    AB", "ab"},
		{"\u0345\u05b0", "", ""},
	} {
		if actual := FoldText(scenario.input); actual != scenario.folded {
			t.Errorf("fold %q: got %q, want %q", scenario.input, actual, scenario.folded)
		}
		if actual := PrepareText(scenario.input); actual != scenario.prepared {
			t.Errorf("prepare %q: got %q, want %q", scenario.input, actual, scenario.prepared)
		}
	}
}

// TestPrepareSimpleQueryBlankProjection retains the original operand before raw tokenization.
func TestPrepareSimpleQueryBlankProjection(t *testing.T) {
	for _, scenario := range []struct{ input, expected string }{
		{"!!!", "! ! !"},
		{"😀", "😀"},
		{"\u0301", "\u0301"},
		{"a!", "a"},
		{"Émile", "emile"},
		{" \t\n", ""},
	} {
		if actual := PrepareQuery(scenario.input, ""); actual != scenario.expected {
			t.Errorf("simple query %q: got %q, want %q", scenario.input, actual, scenario.expected)
		}
	}
}
