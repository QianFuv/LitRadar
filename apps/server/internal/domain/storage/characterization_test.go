package storage

import (
	"strings"
	"testing"
)

// requireDatePrecision checks nil rejection separately from accepted precision text.
func requireDatePrecision(t *testing.T, value, expected string) {
	t.Helper()
	actual := DatePrecision(value)
	if expected == "" {
		if actual != nil {
			t.Fatalf("%q accepted as %q", value, *actual)
		}
		return
	}
	if actual == nil || *actual != expected {
		t.Fatalf("%q precision=%v, expected %q", value, actual, expected)
	}
}

// TestDatePrecisionPreservesPartialGregorianDates fixes year-zero and actual calendar semantics.
func TestDatePrecisionPreservesPartialGregorianDates(t *testing.T) {
	cases := []struct{ value, precision string }{
		{"0000", "year"}, {"9999", "year"}, {"2026", "year"},
		{"0000-01", "month"}, {"2026-12", "month"}, {"2026-00", ""}, {"2026-13", ""},
		{"0000-02-29", "day"}, {"2000-02-29", "day"}, {"1900-02-29", ""}, {"2024-02-29", "day"}, {"2026-02-29", ""},
		{"2026-04-30", "day"}, {"2026-04-31", ""}, {"2026-01-00", ""}, {"2026-12-32", ""},
		{"2026-01-01", "day"}, {"9999-12-31", "day"}, {"2026-1", ""}, {"2026-01-1", ""},
		{"26", ""}, {"10000", ""}, {"2026-01-01T00:00:00Z", ""}, {"2026/01/01", ""}, {"2026-01_01", ""},
	}
	for _, test := range cases {
		requireDatePrecision(t, test.value, test.precision)
	}
}

// TestDatePrecisionNormalizesWhitespaceWithoutWideningDigits fixes normalization and strict ASCII admission.
func TestDatePrecisionNormalizesWhitespaceWithoutWideningDigits(t *testing.T) {
	for _, padding := range []string{" ", "\t\r\n", "\u00a0", "\u2000", "\u2028", "\u3000"} {
		requireDatePrecision(t, padding+"2024-02-29"+padding, "day")
	}
	for _, value := range []string{"", "\u3000", "\ufeff2026", "２０２６", "202é", "202e\u0301", "2026-０1", "2026\xff", "2026-01-\x00x", "2026\u200b", "2026-01-01\nx", strings.Repeat("2", 128)} {
		requireDatePrecision(t, value, "")
	}
}

// TestDatePrecisionReturnsIndependentAcceptedValues checks that later calls cannot mutate earlier output.
func TestDatePrecisionReturnsIndependentAcceptedValues(t *testing.T) {
	year := DatePrecision("2026")
	month := DatePrecision("2026-01")
	day := DatePrecision("2026-01-01")
	if year == nil || month == nil || day == nil {
		t.Fatal("valid precision rejected")
	}
	*year = "changed"
	if *month != "month" || *day != "day" {
		t.Fatal("accepted results share storage")
	}
}
