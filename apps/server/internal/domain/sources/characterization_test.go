package sources

import (
	"encoding/json"
	"math"
	"reflect"
	"testing"
)

// TestNumberRetainsTokensAndUsesStandardConversion checks exact integers and rounded decimals.
func TestNumberRetainsTokensAndUsesStandardConversion(t *testing.T) {
	for _, token := range []string{"0", "-0", "9223372036854775808", "-9223372036854775809", "18446744073709551616.1", "-0e2147483648"} {
		number, err := ParseNumber(json.Number(token))
		if err != nil || number.String() != token {
			t.Fatal(token, number, err)
		}
	}
	number, err := ParseNumber("-1e-2147483648")
	if err != nil || !math.Signbit(number.AsFloat64()) {
		t.Fatal(number, err)
	}
	integer, err := ParseNumber("9007199254740993")
	if value, ok := integer.AsInt64(); err != nil || !ok || value != 9007199254740993 {
		t.Fatal(integer, err)
	}
	if _, err := ParseNumber("1e400"); err != ErrInvalidNumber {
		t.Fatal(err)
	}
}

func TestJsonUsesStandardEncoding(t *testing.T) {
	if encoded, err := Json(map[string]any{"invalid": make(chan int)}); err == nil || encoded != nil {
		t.Fatal(string(encoded), err)
	}
	encoded, err := Json([]any{[]any(nil), map[string]any(nil), "<>&"})
	if err != nil || string(encoded) != `[[],{},"<>&"]` {
		t.Fatal(string(encoded), err)
	}
}

// TestIssueRejectsFloatingYearWithoutReplacingReceiver preserves typed year admission and atomic decoding.
func TestIssueRejectsFloatingYearWithoutReplacingReceiver(t *testing.T) {
	value := IssueDraft{CatalogId: "original", PublicationYear: new(int64(2000))}
	before := IssueDraft{CatalogId: "original", PublicationYear: new(int64(2000))}
	err := value.UnmarshalJSON([]byte(`{"catalog_id":"accepted","publication_year":2026.0}`))
	if err != errContractJson || !reflect.DeepEqual(value, before) {
		t.Fatal(value, err)
	}
	if err := value.UnmarshalJSON([]byte(`{"catalog_id":"accepted","publication_year":2026}`)); err != nil || value.CatalogId != "accepted" || value.PublicationYear == nil || *value.PublicationYear != 2026 {
		t.Fatal("valid integer year was rejected", value, err)
	}
}

// TestProgressFailureDoesNotReplaceReceiver preserves atomic public contract decoding.
func TestProgressFailureDoesNotReplaceReceiver(t *testing.T) {
	for _, raw := range []string{
		`["continue",null]`, `["complete",1]`,
		`{"state":"complete","checkpoint":"x"}`,
	} {
		value := ProviderProgress{State: Continue, Checkpoint: new("original")}
		before := value
		if err := value.UnmarshalJSON([]byte(raw)); err == nil || !reflect.DeepEqual(value, before) {
			t.Fatal(raw, value, err)
		}
	}
	value := ProviderProgress{State: Continue, Checkpoint: new("original")}
	if err := value.UnmarshalJSON([]byte(`{"state":"complete"}`)); err != nil || value.Checkpoint != nil || value.State != Complete {
		t.Fatal(value, err)
	}
}

// TestCanonicalArticleOwnsNormalizedObservations preserves copies and author multiplicity.
func TestCanonicalArticleOwnsNormalizedObservations(t *testing.T) {
	input := ArticleDraft{
		Title: " Title ", PublicationYear: new(int64(2026)), Volume: new(" 1 "),
		Doi: new("NONCANONICAL"), Authors: []ArticleAuthorDraft{{" A "}, {" "}, {"A"}},
		RetractionDois: []string{"10.1234/z", "10.1234/a", "10.1234/z"},
	}
	article := CanonicalArticle(input)
	if article == nil {
		t.Fatal("nonblank title with external identity rejected")
	}
	if !reflect.DeepEqual(article.Authors, []ArticleAuthorDraft{{"A"}, {"A"}}) || !reflect.DeepEqual(article.RetractionDois, []string{"10.1234/a", "10.1234/z"}) {
		t.Fatal(article)
	}
	*article.Volume = "changed"
	*article.PublicationYear = 2000
	article.Authors[0].DisplayName = "changed"
	article.RetractionDois[0] = "changed"
	if *input.Volume != " 1 " || *input.PublicationYear != 2026 || input.Authors[0].DisplayName != " A " || input.RetractionDois[0] != "10.1234/z" {
		t.Fatal("canonical article aliases input observations", input)
	}
}

// TestCanonicalBlankTitleNeedsCanonicalDoi preserves fallback identity admission.
func TestCanonicalBlankTitleNeedsCanonicalDoi(t *testing.T) {
	for _, input := range []ArticleDraft{
		{Title: " ", Pmid: new("123")}, {Title: " ", Doi: new(" 10.1234/a ")},
		{Title: "Title", Date: new("2026-01-01")},
	} {
		if article := CanonicalArticle(input); article != nil {
			t.Fatal("insufficient identity accepted", article)
		}
	}
	if article := CanonicalArticle(ArticleDraft{Title: " ", Doi: new("10.1234/a")}); article == nil || article.Title != "" {
		t.Fatal("canonical DOI title fallback rejected", article)
	}
}
