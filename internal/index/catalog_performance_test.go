package index

import (
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

// catalogLineReference preserves the original rune-based parser as an independent grammar oracle.
func catalogLineReference(line string) ([]string, error) {
	characters := []rune(strings.TrimRight(line, "\r"))
	values := []string{}
	var current strings.Builder
	isQuoted := false
	for position := 0; position < len(characters); position++ {
		character := characters[position]
		switch {
		case character == '"' && isQuoted && position+1 < len(characters) && characters[position+1] == '"':
			current.WriteRune('"')
			position++
		case character == '"':
			isQuoted = !isQuoted
		case character == ',' && !isQuoted:
			values = append(values, strings.TrimSpace(current.String()))
			current.Reset()
		default:
			current.WriteRune(character)
		}
	}
	if isQuoted {
		return nil, errors.New("catalog CSV row has an unterminated quoted field")
	}
	return append(values, strings.TrimSpace(current.String())), nil
}

// FuzzCatalogLine compares exact fields and diagnostics for arbitrary bytes, quotes and whitespace.
func FuzzCatalogLine(f *testing.F) {
	for _, line := range []string{"", "a,,b,", `"Journal, 中文","A ""quoted"" title",tail`, " a\u00a0,\t b \r\r", "a\"b\"c,d", `"unterminated`, "\xff\xff,\xc3,a\"\xfe\""} {
		f.Add(line)
	}
	f.Fuzz(func(t *testing.T, line string) {
		if len(line) > 8192 {
			t.Skip()
		}
		expected, expectedFailure := catalogLineReference(line)
		actual, failure := parseCatalogLine(line)
		if !reflect.DeepEqual(actual, expected) || (failure == nil) != (expectedFailure == nil) || failure != nil && failure.Error() != expectedFailure.Error() {
			t.Fatal("catalog grammar changed", line, actual, expected, failure, expectedFailure)
		}
	})
}

// BenchmarkParseCatalogCsv measures complete validation of the maintained provider catalogs.
func BenchmarkParseCatalogCsv(b *testing.B) {
	for _, name := range []string{"english_journals", "chinese_journals", "ccf_computer_journals"} {
		body, err := os.ReadFile(filepath.Join("..", "..", "assets", "meta", name+".csv"))
		if err != nil {
			b.Fatal(err)
		}
		text := string(body)
		b.Run(name, func(b *testing.B) {
			b.ReportAllocs()
			for b.Loop() {
				entries, err := ParseCatalogCsv(text)
				if err != nil || len(entries) == 0 {
					b.Fatal("maintained catalog failed admission", err)
				}
			}
		})
	}
}
