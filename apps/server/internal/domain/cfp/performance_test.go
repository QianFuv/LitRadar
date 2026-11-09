package cfp

import (
	"strings"
	"testing"
)

// TestPatternMatchesRetainsCaptureSemantics protects Unicode assertions and empty matches.
func TestPatternMatchesRetainsCaptureSemantics(t *testing.T) {
	for _, pattern := range []string{`(?i)submission|deadline`, `\bcall\b`, `\\b`, `\s`, `\w+`, `(a)?`, `^$`, `(?P<boundary>)call`, `\b`, `(\b)?call`, `(?m)^\bcall\b`, `\bcall\b|^recall$`} {
		for _, text := range []string{"", "Submission deadline", "recall", "call", "call call", "文call文 call", "recall call", "calls recall", "\ncall", "\u200dcall", `\b`, "\u0085"} {
			expected := len(PatternCaptures(pattern, text)) > 0
			if actual := PatternMatches(pattern, text); actual != expected {
				t.Errorf("pattern %q text %q: got %v, want %v", pattern, text, actual, expected)
			}
		}
	}
}

// FuzzPatternMatches agrees with full capture enumeration for Unicode and zero-width boundaries.
func FuzzPatternMatches(f *testing.F) {
	patterns := []string{`\bcall\b`, `\b`, `\b\w+\b`, `(\b)?call`, `(?m)^\bcall\b`, `\bcall\b|^recall$`, `(?P<boundary>call)`, `(?i)\b(submission|deadline)\b`, `a*\b`, `(?:\b|call)*`}
	for index := range patterns {
		for _, value := range []string{"", "recall call", "文call文 call", "\u200dcall", "\ncall", "a\xffcall", "Submission deadline"} {
			f.Add(uint8(index), value)
		}
	}
	f.Fuzz(func(t *testing.T, selector uint8, value string) {
		if len(value) > 4096 {
			t.Skip()
		}
		pattern := patterns[int(selector)%len(patterns)]
		expected := len(PatternCaptures(pattern, value)) > 0
		if actual := PatternMatches(pattern, value); actual != expected {
			t.Fatalf("pattern %q value %q: got %v, want %v", pattern, value, actual, expected)
		}
	})
}

// BenchmarkPatternMatch measures boolean checks over a publisher's repeated labels.
func BenchmarkPatternMatch(b *testing.B) {
	text := strings.Repeat("Submission deadline: 31 December 2026. ", 1000)
	for _, pattern := range []string{`submission|deadline`, `\bsubmission\b`, `absent`} {
		b.Run(pattern, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				matches(pattern, text)
			}
		})
	}
}

// BenchmarkBoundaryMatchFallback compares first-match acceptance and rejected-boundary fallback costs.
func BenchmarkBoundaryMatchFallback(b *testing.B) {
	for _, scenario := range []struct {
		name string
		text string
	}{{"short-valid", "call"}, {"short-rejected", "recall"}, {"later-valid", "recall " + strings.Repeat("call ", 1000)}, {"all-rejected", strings.Repeat("recall ", 1000)}, {"absent", "No relevant topic"}} {
		for _, isReference := range []bool{false, true} {
			name := scenario.name + "/optimized"
			if isReference {
				name = scenario.name + "/reference"
			}
			b.Run(name, func(b *testing.B) {
				b.ReportAllocs()
				for b.Loop() {
					if isReference {
						PatternCaptures(`\bcall\b`, scenario.text)
					} else {
						PatternMatches(`\bcall\b`, scenario.text)
					}
				}
			})
		}
	}
}
