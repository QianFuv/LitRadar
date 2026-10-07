package api

import "testing"

// TestDebugStringPreservesSerdeEscapes checks diagnostic spelling and Unicode admission.
func TestDebugStringPreservesSerdeEscapes(t *testing.T) {
	for _, scenario := range []struct{ value, expected string }{
		{"", `""`},
		{"plain <>&中文", `"plain <>&中文"`},
		{"\x00\n\r\t\\\"", `"\0\n\r\t\\\""`},
		{"\b\f\x1f\x7f", `"\u{8}\u{c}\u{1f}\u{7f}"`},
		{"a\u0301\u20dd", `"a\u{301}\u{20dd}"`},
		{"\uff9e", `"\u{ff9e}"`},
		{"\u00a0\u200d\u2028", `"\u{a0}\u{200d}\u{2028}"`},
		{"🙂", `"🙂"`},
		{string([]byte{255, 192, 128}), "\"���\""},
	} {
		if result := DebugString(scenario.value); result != scenario.expected {
			t.Fatal(scenario.value, result, scenario.expected)
		}
	}
}
