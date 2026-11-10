package search

import (
	"reflect"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
)

// TestTokenizeOracleGoldens retains tokens confirmed against the legacy v0.7.1 DLL.
func TestTokenizeOracleGoldens(t *testing.T) {
	cases := []struct {
		name, input string
		expected    []string
	}{
		{"empty", "", []string{}},
		{"controls", "\x00\x01\t\n\r\v\f\x1a\x1f\x7f", []string{}},
		{"byte classes", "AB12cd 00123 alpha ALPHA", []string{"ab", "12", "cd", "00123", "alpha", "alpha"}},
		{"mixed", "深度学习 Deep-Learning for DOI 10.1000/ABC_9 «测试» Ünïcode \"q\"", []string{"深", "度", "学", "习", "deep", "-", "learning", "for", "doi", "10", ".", "1000", "/", "abc", "_", "9", "«", "测", "试", "»", "Ü", "n", "ï", "code", "\"", "q", "\""}},
		{"apostrophe", "O'Brien 的研究", []string{"o", "'", "brien", "的", "研", "究"}},
		{"four byte", "𠀀😀中文", []string{"𠀀", "😀", "中", "文"}},
		{"non ascii case", "Résumé École cafe\u0301 CAFÉ", []string{"r", "é", "sum", "é", "É", "cole", "cafe", "\u0301", "caf", "É"}},
		{"isolated continuation", "\x80\xbf", []string{"\x80", "\xbf"}},
		{"overlong", "\xc0\xaf", []string{"\xc0\xaf"}},
		{"oversized lead", "\xf8\x80\x80\x80", []string{"\xf8\x80\x80\x80"}},
		{"invalid trailing ascii", "\xe1ab", []string{"\xe1ab"}},
	}
	for _, item := range cases {
		t.Run(item.name, func(t *testing.T) {
			if actual := Tokenize(item.input); !reflect.DeepEqual(actual, item.expected) {
				t.Fatalf("tokens=%q expected=%q", actual, item.expected)
			}
		})
	}
}

// TestTokenizeClampsTruncatedLeads protects the deliberate difference from native out-of-bounds reads.
func TestTokenizeClampsTruncatedLeads(t *testing.T) {
	for _, value := range []string{"\xc3", "\xe2\x82", "\xf0\x9f", "\xff"} {
		t.Run(strings.ReplaceAll(value, "/", "_"), func(t *testing.T) {
			if actual := Tokenize(value); !reflect.DeepEqual(actual, []string{value}) {
				t.Fatalf("truncated bytes %x became %x", value, actual)
			}
		})
	}
}

// TestTokenizeAllAscii checks every control, digit, letter and punctuation byte.
func TestTokenizeAllAscii(t *testing.T) {
	for character := byte(0); character < 128; character++ {
		expected := []string{string(character)}
		if character <= 32 || character == 127 {
			expected = []string{}
		} else if character >= 'A' && character <= 'Z' {
			expected = []string{string(character + 32)}
		}
		if actual := Tokenize(string(character)); !reflect.DeepEqual(actual, expected) {
			t.Fatalf("byte %x: %q expected %q", character, actual, expected)
		}
	}
}

// TestPrepareQueryPreservesGrammar checks phrases, operators, columns, prefix and distance syntax.
func TestPrepareQueryPreservesGrammar(t *testing.T) {
	cases := []struct{ value, expected string }{
		{`研究 AND title:AB12*`, `"研 究" AND title:"ab 12"*`},
		{`"O'Brien" OR "a""b"`, `"o brien" OR "a b"`},
		{`{title abstract_text}:NEAR("研究" abc*, 12)`, `{title abstract_text}:NEAR("研 究" "abc"*, 12)`},
		{`"title":École NOT alpha`, `"title":"ecole" NOT "alpha"`},
		{`^alpha + beta`, `^"alpha" + "beta"`},
		{`Résumé AND "late`, `Résumé AND "late`},
		{"\x80", `""`},
	}
	for _, item := range cases {
		if actual := PrepareQuery(item.value, domain.SearchAdvanced); actual != item.expected {
			t.Errorf("%q: %q expected %q", item.value, actual, item.expected)
		}
	}
	if actual := Phrase(`10.1000/ABC_9 "q"`); actual != `"10 1000 abc 9 q"` {
		t.Fatalf("phrase=%s", actual)
	}
}
