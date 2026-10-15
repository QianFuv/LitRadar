package jsonvalue

import (
	"encoding/json"
	"io"
	"strconv"
	"strings"
	"testing"
	"unicode/utf8"
)

// FuzzValidationMatchesDecoder checks compatibility against independently decoded tokens.
func FuzzValidationMatchesDecoder(f *testing.F) {
	for _, value := range []string{
		`null`, `{"title":"文献 [1e400]","year":2026,"open":true}`, `{"x":1e400,"x":0}`,
		`"\\\"[{1e400}]"`, `"\ud800"`, `"\ud83d\ude00"`, `1e-400`, `-0e999999`,
		strings.Repeat(`[{"x":`, 63) + `0` + strings.Repeat(`}]`, 63),
		strings.Repeat(`[`, 128) + `0` + strings.Repeat(`]`, 128),
	} {
		f.Add(value)
	}
	f.Fuzz(func(t *testing.T, value string) {
		expected := utf8.ValidString(value) && validJsonStringEscapes(value) && json.Valid([]byte(value)) && validDecodedTokens(value)
		if actual := ValidJson(value); actual != expected {
			t.Fatalf("validation for %q: got %v, want %v", value, actual, expected)
		}
	})
}

// validDecodedTokens uses the standard decoder as the reference for depth and number limits.
func validDecodedTokens(value string) bool {
	decoder := json.NewDecoder(strings.NewReader(value))
	decoder.UseNumber()
	depth := 0
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			return true
		}
		if err != nil {
			return false
		}
		switch token := token.(type) {
		case json.Delim:
			if token == '[' || token == '{' {
				depth++
				if depth >= 128 {
					return false
				}
			} else {
				depth--
			}
		case json.Number:
			if _, err := strconv.ParseFloat(string(token), 64); err != nil {
				return false
			}
		}
	}
}

// BenchmarkValidJson measures settings and article-batch validation without fixture setup costs.
func BenchmarkValidJson(b *testing.B) {
	for _, scenario := range []struct{ name, value string }{
		{"Settings", `{"enabled":true,"limit":50,"name":"文献追踪"}`},
		{"Articles", `[` + strings.Repeat(`{"title":"文献 [1e400]","year":2026,"doi":"10.1234/example","open":true},`, 999) + `null]`},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			b.ReportAllocs()
			b.SetBytes(int64(len(scenario.value)))
			for b.Loop() {
				if !ValidJson(scenario.value) {
					b.Fatal("valid fixture rejected")
				}
			}
		})
	}
}
