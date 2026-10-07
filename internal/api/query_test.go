package api

import (
	"strings"
	"testing"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/query"
)

func TestArticleQueryRetainsRepeatedFiltersAndLastScalar(t *testing.T) {
	database, params, err := parseArticleQuery("&&db=first&db=+&journal_id=%2B9007199254740993&journal_id=&area=A&area=+&abs_rating=4%2A&abs_rating=&q=old&q=new%3Bvalue&search_mode=ADVANCED&in_press=YES&include_total=off&limit=10&limit=20")
	if err != nil {
		t.Fatal(err)
	}
	assertRepeatedArticleFilters(t, database, params)
	if *params.Query != "new;value" || params.SearchMode != domain.SearchAdvanced || !*params.InPress || *params.IncludeTotal || params.Limit != 20 {
		t.Fatalf("query semantics: %+v", params)
	}
}

func TestArticleQueryRejectsInvalidEncodingAndKeepsValidationOrder(t *testing.T) {
	for _, scenario := range []struct{ name, raw, detail string }{
		{"percent", "q=%", "Invalid query encoding"},
		{"utf8", "q=%FF", "Invalid query encoding"},
		{"invalid unknown key", "%FE=1", "Invalid query encoding"},
		{"filter count before lengths", "db=" + strings.Repeat("a", 256) + "&" + strings.Repeat("area=&", 501), "search filters must contain at most 500 items"},
		{"earlier duplicate length", "q=" + strings.Repeat("中", 2049) + "&q=ok", "q must be at most 2048 characters"},
		{"journal before year", "year=no&journal_id=no", "Invalid integer for journal_id"},
		{"issue before boolean", "in_press=maybe&issue_id=no", "Invalid integer for issue_id"},
		{"mode before limit", "limit=no&search_mode=raw", "search_mode must be simple or advanced"},
		{"ascii mode", "search_mode=S%C4%B0MPLE", "search_mode must be simple or advanced"},
		{"overflow", "year=9223372036854775808", "Invalid integer for year"},
		{"boolean", "include_total=2", "Invalid boolean for include_total"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			_, _, err := parseArticleQuery(scenario.raw)
			if err == nil || err.status != 400 || err.detail != scenario.detail {
				t.Fatalf("got %v; want %s", err, scenario.detail)
			}
		})
	}
	_, params, err := parseArticleQuery("q=" + strings.Repeat("中", 2048) + "&year=+&ignored=" + strings.Repeat("a", 10000))
	if err != nil || params.Year != nil || params.Limit != 50 || *params.Sort != "date:desc" {
		t.Fatalf("defaults and Unicode length: %+v %v", params, err)
	}
}

func TestWeeklyQueryUsesItsOwnRequiredFieldsAndValidationOrder(t *testing.T) {
	for _, scenario := range []struct{ raw, detail string }{
		{"journal_id=bad", "db is required"},
		{"db=x", "journal_id is required"},
		{"db=x&journal_id=0", "journal_id must be greater than 0"},
		{"db=x&journal_id=1", "window_end is required"},
		{"db=x&journal_id=1&window_end=now&limit=no", "Invalid integer for limit"},
	} {
		_, err := parseWeeklyArticleQuery(scenario.raw)
		if err == nil || err.detail != scenario.detail {
			t.Fatalf("%s: got %v; want %s", scenario.raw, err, scenario.detail)
		}
	}
	params, err := parseWeeklyArticleQuery("db=x&journal_id=1&window_end=now&" + strings.Repeat("area=a&", 501))
	if err != nil || params.Limit != 50 || params.WindowEnd != "now" {
		t.Fatalf("weekly inherited unrelated search validation: %+v %v", params, err)
	}
}

// assertRepeatedArticleFilters checks repeated filter values without losing large integer precision.
func assertRepeatedArticleFilters(t *testing.T, database *string, params query.ArticleListParams) {
	t.Helper()
	if database != nil || len(params.JournalId) != 1 || params.JournalId[0] != 9007199254740993 || len(params.Area) != 2 || params.Area[1] != " " || len(params.Ratings.AbsRating) != 2 || params.Ratings.AbsRating[1] != "" {
		t.Fatalf("filter/scalar semantics: %+v %+v", database, params)
	}
}
