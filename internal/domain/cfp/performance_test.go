package cfp

import (
	"strings"
	"testing"
)

// TestPatternMatchesRetainsCaptureSemantics protects Unicode assertions and empty matches.
func TestPatternMatchesRetainsCaptureSemantics(t *testing.T) {
	for _, pattern := range []string{`(?i)submission|deadline`, `\bcall\b`, `\\b`, `\s`, `\w+`, `(a)?`, `^$`, `(?P<boundary>)call`} {
		for _, text := range []string{"", "Submission deadline", "recall", "call", "call call", "文call文 call", "\u200dcall", `\b`, "\u0085"} {
			expected := len(PatternCaptures(pattern, text)) > 0
			if actual := PatternMatches(pattern, text); actual != expected {
				t.Errorf("pattern %q text %q: got %v, want %v", pattern, text, actual, expected)
			}
		}
	}
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
