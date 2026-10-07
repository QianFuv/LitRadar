package citation

import (
	"errors"
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
)

func TestXmlEscapingKeepsAllowedAndForbiddenCharacters(t *testing.T) {
	output := boundedText{maximum: 1000}
	output.xml("&<>\"'\x00\t\n\r\x1f\u007f\ufffe\uffff😀")
	got, err := output.finish()
	if err != nil || got != "&amp;&lt;&gt;&quot;&apos; \t\n\r \u007f  😀" {
		t.Fatalf("XML character policy changed: %q %v", got, err)
	}
}

func TestBibtexKeyAndAuthorOrderingRemainStructural(t *testing.T) {
	doi := ".-中文a/Z_9"
	title := "title{%}\n"
	result, err := Bibtex([]domain.FavoriteCitation{{Doi: &doi, Title: &title, Authors: []string{"first\nline", "second"}}, {}}, 10000)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(result, "@article{aZ91,\n") || !strings.Contains(result, "author = {first line and second}") || !strings.Contains(result, "@article{favorite2,\n") {
		t.Fatal("key/author/fallback order changed", result)
	}
	if !strings.Contains(result, `title\char123{}\%\char125{} `) {
		t.Fatal("structural escaping changed", result)
	}
}

func TestCitationByteLimitIsInclusiveAndFailureReturnsNoPartialExport(t *testing.T) {
	title := "中&{}"
	articles := []domain.FavoriteCitation{{Title: &title, Authors: []string{"author"}}}
	for _, serialize := range []func([]domain.FavoriteCitation, int) (string, error){Bibtex, Ris, EndnoteXml} {
		original, err := serialize(articles, 10000)
		if err != nil {
			t.Fatal(err)
		}
		exact, err := serialize(articles, len(original))
		if err != nil || exact != original {
			t.Fatal("inclusive byte boundary changed", err)
		}
		failed, err := serialize(articles, len(original)-1)
		if !errors.Is(err, ErrOutputLimit) || failed != "" {
			t.Fatal("partial export escaped failed limit", err)
		}
	}
}
