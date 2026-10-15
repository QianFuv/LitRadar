package citation

import (
	"errors"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

// TestExportMalformedUtf8AndByteLimits protects replacement width and inclusive export bounds.
func TestExportMalformedUtf8AndByteLimits(t *testing.T) {
	for _, format := range []struct {
		name   string
		export func([]domain.FavoriteCitation, int) (string, error)
	}{{"Bibtex", Bibtex}, {"Ris", Ris}, {"Xml", EndnoteXml}} {
		t.Run(format.name, func(t *testing.T) {
			for _, title := range []string{"safe text", "文献\ufffdtext", "before\xff\xc3after", "\xff<&{}\\\n\xff", "\xf0\x9f"} {
				clean := string([]rune(title))
				expected, err := format.export([]domain.FavoriteCitation{{Title: &clean}}, 1<<20)
				if err != nil {
					t.Fatal(err)
				}
				articles := []domain.FavoriteCitation{{Title: &title}}
				actual, err := format.export(articles, len(expected))
				if err != nil || actual != expected {
					t.Fatalf("title %q: got %q, want %q: %v", title, actual, expected, err)
				}
				if actual, err := format.export(articles, len(expected)-1); actual != "" || !errors.Is(err, ErrOutputLimit) {
					t.Fatalf("oversized export published: %q, %v", actual, err)
				}
			}
		})
	}
}

// BenchmarkExportText measures long safe fields and dense escaping in each supported format.
func BenchmarkExportText(b *testing.B) {
	for _, workload := range []struct{ name, text string }{
		{"Ascii", strings.Repeat("Research title and abstract. ", 4096)},
		{"Unicode", strings.Repeat("文献研究与分析。", 4096)},
		{"Escapes", strings.Repeat("<&{}\\\n", 4096)},
	} {
		for _, format := range []struct {
			name   string
			export func([]domain.FavoriteCitation, int) (string, error)
		}{{"Bibtex", Bibtex}, {"Ris", Ris}, {"Xml", EndnoteXml}} {
			b.Run(workload.name+"/"+format.name, func(b *testing.B) {
				articles := []domain.FavoriteCitation{{Title: &workload.text}}
				b.ReportAllocs()
				b.SetBytes(int64(len(workload.text)))
				for b.Loop() {
					if _, err := format.export(articles, 8*1024*1024); err != nil {
						b.Fatal(err)
					}
				}
			})
		}
	}
}
