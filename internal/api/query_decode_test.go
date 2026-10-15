package api

import (
	"net/url"
	"reflect"
	"strconv"
	"strings"
	"testing"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/transport"
)

// referenceFormDecode retains the original permissive byte-decoding and UTF-8 replacement path.
func referenceFormDecode(value string) string {
	decoded := make([]byte, 0, len(value))
	for index := 0; index < len(value); index++ {
		switch value[index] {
		case '+':
			decoded = append(decoded, ' ')
		case '%':
			if index+2 < len(value) {
				if number, err := strconv.ParseUint(value[index+1:index+3], 16, 8); err == nil {
					decoded = append(decoded, byte(number))
					index += 2
					continue
				}
			}
			decoded = append(decoded, '%')
		default:
			decoded = append(decoded, value[index])
		}
	}
	return transport.LossyUtf8(decoded)
}

// referenceExtractQuery retains ignored-field handling and validation order before allocation optimizations.
func referenceExtractQuery(raw string, fields map[string]queryKind) (typedQuery, *apiError) {
	values := typedQuery{}
	for _, part := range strings.Split(raw, "&") {
		if part == "" {
			continue
		}
		name, value, _ := strings.Cut(part, "=")
		name, value = referenceFormDecode(name), referenceFormDecode(value)
		kind, known := fields[name]
		if !known {
			continue
		}
		if _, exists := values[name]; exists {
			return nil, queryRejection("duplicate field `" + name + "`")
		}
		parsed, failure, isHandled := decodeQueryValue(kind, name, value)
		if failure != nil {
			return nil, failure
		}
		if isHandled {
			values[name] = parsed
		}
	}
	return values, nil
}

// referenceQueryPairs preserves strict encoding rejection for every field, including unknown fields.
func referenceQueryPairs(raw string) (queryPairs, *apiError) {
	pairs := queryPairs{}
	for _, part := range strings.Split(raw, "&") {
		if part == "" {
			continue
		}
		name, value, _ := strings.Cut(part, "=")
		name, nameError := url.QueryUnescape(name)
		value, valueError := url.QueryUnescape(value)
		if nameError != nil || valueError != nil || !utf8.ValidString(name) || !utf8.ValidString(value) {
			return nil, badRequest("Invalid query encoding")
		}
		pairs = append(pairs, queryPair{name, value})
	}
	return pairs, nil
}

// assertQueryDecoding preserves field values, duplicate/encoding failures, and strict versus permissive behavior.
func assertQueryDecoding(t *testing.T, raw string) {
	t.Helper()
	fields := map[string]queryKind{"db": queryText, "q": queryText, "limit": queryUnsigned, "year": queryInteger, "active": queryBoolean, "unhandled": 255}
	actual, failure := extractQuery(raw, fields)
	want, expectedFailure := referenceExtractQuery(raw, fields)
	if !reflect.DeepEqual(actual, want) || !reflect.DeepEqual(failure, expectedFailure) {
		t.Fatalf("typed query changed for %q: %v, %v; expected %v, %v", raw, actual, failure, want, expectedFailure)
	}
	pairs, pairFailure := parseQueryPairs(raw)
	expectedPairs, expectedPairFailure := referenceQueryPairs(raw)
	if !reflect.DeepEqual(pairs, expectedPairs) || !reflect.DeepEqual(pairFailure, expectedPairFailure) {
		t.Fatalf("strict query changed for %q: %v, %v; expected %v, %v", raw, pairs, pairFailure, expectedPairs, expectedPairFailure)
	}
	if actual, expected := formDecode(raw), referenceFormDecode(raw); actual != expected {
		t.Fatalf("form decoding changed for %q: %q; expected %q", raw, actual, expected)
	}
}

// TestQueryDecodingCompatibility checks ordering, escapes, discarded values, and raw invalid UTF-8.
func TestQueryDecodingCompatibility(t *testing.T) {
	for _, raw := range []string{
		"", "&&", "db=fixture.sqlite&limit=50&active=true", "q=中文&year=-2026", "q=first&q=%FF", "q=first&%71=second",
		"limit=no&active=bad", "active=bad&limit=no", "ignored=%FF%&q=kept", "unhandled=x&unhandled=y", "q=%+0+%2B%3d&ignored=%gg",
		"q=%e4%b8%ad%e6%96%87", "q=\xff\xfe", "\xff=value&q=\xe4\xb8", "limit=%2B1&year=-9223372036854775809", "active=true&active=false",
		"unknown=" + strings.Repeat("%FF+", 1000) + "&q=after", "q=first&q=" + strings.Repeat("%FF+", 1000),
	} {
		assertQueryDecoding(t, raw)
	}
}

// FuzzQueryDecoding compares both parser contracts with their original implementations.
func FuzzQueryDecoding(f *testing.F) {
	for _, raw := range []string{"q=plain&db=fixture.sqlite", "q=%FF&ignored=%+%E4%B8", "q=1&%71=2", "active=no&limit=%2B1", "q=\xff\xe4\xb8"} {
		f.Add(raw)
	}
	f.Fuzz(func(t *testing.T, raw string) {
		if len(raw) > 8192 {
			t.Skip()
		}
		assertQueryDecoding(t, raw)
	})
}

// BenchmarkQueryDecoding compares request shapes against the original copying paths.
func BenchmarkQueryDecoding(b *testing.B) {
	fields := map[string]queryKind{"db": queryText, "q": queryText, "limit": queryUnsigned, "active": queryBoolean}
	for _, scenario := range []struct{ name, raw string }{
		{"Ordinary", "db=fixture.sqlite&q=distributed-systems&limit=50&active=true"},
		{"Ignored", "ignored=" + strings.Repeat("%FF+", 10000) + "&q=kept"},
		{"RepeatedFilters", strings.Repeat("area=Information+Systems&", 500)},
	} {
		for _, usesReference := range []bool{true, false} {
			b.Run(scenario.name+"/reference="+strconv.FormatBool(usesReference), func(b *testing.B) {
				b.ReportAllocs()
				for b.Loop() {
					var failure *apiError
					if scenario.name == "RepeatedFilters" {
						if usesReference {
							_, failure = referenceQueryPairs(scenario.raw)
						} else {
							_, failure = parseQueryPairs(scenario.raw)
						}
					} else if usesReference {
						_, failure = referenceExtractQuery(scenario.raw, fields)
					} else {
						_, failure = extractQuery(scenario.raw, fields)
					}
					if failure != nil {
						b.Fatal(failure)
					}
				}
			})
		}
	}
}
