package sources

import (
	"bytes"
	"encoding/json"
	"math"
	"reflect"
	"testing"
)

// TestNumberVariantsPreserveIdentity distinguishes integer tokens and signed floating zero.
func TestNumberVariantsPreserveIdentity(t *testing.T) {
	for _, entry := range []struct {
		token string
		kind  byte
	}{
		{"0", 'u'}, {"-0", 'f'}, {"9223372036854775808", 'u'},
		{"-9223372036854775808", 'i'}, {"-9223372036854775809", 'f'},
		{"18446744073709551616.1", 'f'}, {"-0e2147483648", 'f'},
	} {
		number, err := ParseNumber(json.Number(entry.token))
		if err != nil || number.kind != entry.kind {
			t.Fatal(entry.token, number, err)
		}
	}
	number, err := ParseNumber("-1e-2147483648")
	if err != nil || math.Float64bits(number.AsFloat64()) != 1<<63 {
		t.Fatal("negative exponent overflow lost signed zero", number, err)
	}
}

// TestJsonFailurePreservesOrderedPrefix pins sorted-key admission before value errors.
func TestJsonFailurePreservesOrderedPrefix(t *testing.T) {
	var output bytes.Buffer
	output.WriteString("prefix:")
	err := appendJson(&output, map[string]any{"z": make(chan int), "a": []any{true, "\xff"}})
	if err == nil || err.Error() != "source JSON string is not UTF-8" || output.String() != `prefix:{"a":[true,` {
		t.Fatal(output.String(), err)
	}
	encoded, err := Json([]any{[]any(nil), map[string]any(nil)})
	if err != nil || string(encoded) != "[[],{}]" {
		t.Fatal(string(encoded), err)
	}
}

// TestContractFailureRetainsAllocatedPartialFields pins internal decoding mutation order.
func TestContractFailureRetainsAllocatedPartialFields(t *testing.T) {
	var value struct {
		Title string   `json:"title"`
		Year  *int64   `json:"year"`
		Names []string `json:"names"`
	}
	err := decodeContractStruct([]byte(`{"title":"accepted","year":2026.0,"names":[]}`), &value)
	if err != errContractJson || value.Title != "accepted" || value.Year == nil {
		t.Fatal(value, err)
	}
	if *value.Year != 0 || value.Names != nil {
		t.Fatal("later fields changed after invalid year", value)
	}
}

// TestProgressFailureDoesNotReplaceReceiver preserves atomic public contract decoding.
func TestProgressFailureDoesNotReplaceReceiver(t *testing.T) {
	for _, raw := range []string{
		`["continue",null]`, `["complete",1]`,
		`{"state":"continue","checkpoint":"x","\u0063heckpoint":"y"}`,
		`{"state":"complete","checkpoint":"x"}`,
	} {
		value := ProviderProgress{State: Continue, Checkpoint: new("original")}
		before := value
		if err := value.UnmarshalJSON([]byte(raw)); err == nil || !reflect.DeepEqual(value, before) {
			t.Fatal(raw, value, err)
		}
	}
	value := ProviderProgress{State: Continue, Checkpoint: new("original")}
	if err := value.UnmarshalJSON([]byte(`["complete"]`)); err != nil || value.Checkpoint != nil || value.State != Complete {
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
