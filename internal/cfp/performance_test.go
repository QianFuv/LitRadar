package cfp

import "testing"

// BenchmarkDateClauses measures repeated notice extraction without network or DOM setup.
func BenchmarkDateClauses(b *testing.B) {
	body := "Research topics. Submission deadline: 31 December 2026. Notification: 1 March 2027. Final decision: 1 April 2027."
	b.ReportAllocs()
	for b.Loop() {
		if dateClauses(body) == "" {
			b.Fatal("missing dates")
		}
	}
}
