package transport

import (
	"bytes"
	"testing"

	"golang.org/x/text/encoding/unicode"
)

// BenchmarkLossyUtf8 measures response decoding for valid and malformed upstream text.
func BenchmarkLossyUtf8(b *testing.B) {
	for _, scenario := range []struct {
		name string
		body []byte
	}{
		{"Url", []byte("https://publisher.example/articles?query=research")},
		{"Html", bytes.Repeat([]byte("<p>Research 文献摘要</p>\n"), 8192)},
		{"Malformed", bytes.Repeat([]byte("text\xff\xe6\x83"), 128)},
	} {
		b.Run(scenario.name, func(b *testing.B) {
			b.ReportAllocs()
			b.SetBytes(int64(len(scenario.body)))
			for b.Loop() {
				LossyUtf8(scenario.body)
			}
		})
	}
}

// FuzzLossyUtf8MatchesDecoder preserves the decoder's replacement groups for arbitrary bytes.
func FuzzLossyUtf8MatchesDecoder(f *testing.F) {
	for _, body := range [][]byte{nil, []byte("ASCII"), []byte("\ufeff文献😀"), {0xe6, 0x83, 0xff, 0xff}, {0xf0, 0x9f}, {0xed, 0xa0, 0x80}} {
		f.Add(body)
	}
	f.Fuzz(func(t *testing.T, body []byte) {
		expected, _ := unicode.UTF8.NewDecoder().Bytes(body)
		if actual := LossyUtf8(body); actual != string(expected) {
			t.Fatalf("decoded %x as %q, want %q", body, actual, expected)
		}
	})
}

// TestLossyUtf8ReturnsIndependentText protects returned text from later response-buffer reuse.
func TestLossyUtf8ReturnsIndependentText(t *testing.T) {
	body := []byte("文献 response")
	actual := LossyUtf8(body)
	for index := range body {
		body[index] = 'x'
	}
	if actual != "文献 response" {
		t.Fatal("decoded text aliases the mutable response buffer")
	}
}
