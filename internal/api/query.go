package api

import (
	"fmt"
	"net/url"
	"strconv"
	"strings"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/query"
)

type queryPair struct{ name, value string }
type queryPairs []queryPair

func parseQueryPairs(raw string) (queryPairs, *apiError) {
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

func (pairs queryPairs) values(key string) []string {
	values := []string{}
	for _, pair := range pairs {
		if pair.name == key {
			values = append(values, pair.value)
		}
	}
	return values
}

func (pairs queryPairs) value(key string) *string {
	for index := len(pairs) - 1; index >= 0; index-- {
		if pairs[index].name == key {
			value := strings.TrimSpace(pairs[index].value)
			if value != "" {
				return &value
			}
			return nil
		}
	}
	return nil
}

func parseInteger(key, value string) (int64, *apiError) {
	number, err := strconv.ParseInt(strings.TrimSpace(value), 10, 64)
	if err != nil {
		return 0, badRequest("Invalid integer for " + key)
	}
	return number, nil
}

func (pairs queryPairs) integer(key string) (*int64, *apiError) {
	if value := pairs.value(key); value != nil {
		number, err := parseInteger(key, *value)
		return &number, err
	}
	return nil, nil
}

func (pairs queryPairs) boolean(key string) (*bool, *apiError) {
	value := pairs.value(key)
	if value == nil {
		return nil, nil
	}
	var result bool
	switch asciiLower(*value) {
	case "1", "true", "on", "yes":
		result = true
	case "0", "false", "off", "no":
		result = false
	default:
		return nil, badRequest("Invalid boolean for " + key)
	}
	return &result, nil
}

func validateCharacters(name, value string, maximum int) *apiError {
	if utf8.RuneCountInString(value) > maximum {
		return badRequest(fmt.Sprintf("%s must be at most %d characters", name, maximum))
	}
	return nil
}

func (pairs queryPairs) validateArticle() *apiError {
	count := 0
	for _, pair := range pairs {
		switch pair.name {
		case "journal_id", "area", "utd_rating", "abs_rating", "fms_rating", "fmscn_rating":
			count++
		}
	}
	if count > 500 {
		return badRequest("search filters must contain at most 500 items")
	}
	for _, pair := range pairs {
		maximum := 2048
		switch pair.name {
		case "db":
			maximum = 255
		case "journal_id", "area", "utd_rating", "abs_rating", "fms_rating", "fmscn_rating", "date_from", "date_to", "doi", "pmid", "q", "search_mode", "sort", "cursor":
		default:
			continue
		}
		if err := validateCharacters(pair.name, pair.value, maximum); err != nil {
			return err
		}
	}
	return nil
}

func (pairs queryPairs) ratings() domain.JournalRatings {
	return domain.JournalRatings{UtdRating: pairs.values("utd_rating"), AbsRating: pairs.values("abs_rating"), FmsRating: pairs.values("fms_rating"), FmscnRating: pairs.values("fmscn_rating")}
}

// parseArticleQuery preserves filter, scalar, search and pagination conversion order.
func parseArticleQuery(raw string) (*string, query.ArticleListParams, *apiError) {
	params := query.DefaultArticleListParams()
	pairs, err := parseQueryPairs(raw)
	if err != nil {
		return nil, params, err
	}
	if err = pairs.validateArticle(); err != nil {
		return nil, params, err
	}
	if err = parseArticleJournals(pairs, &params); err != nil {
		return nil, params, err
	}
	params.Area, params.Ratings = pairs.values("area"), pairs.ratings()
	if params.IssueId, err = pairs.integer("issue_id"); err != nil {
		return nil, params, err
	}
	if params.Year, err = pairs.integer("year"); err != nil {
		return nil, params, err
	}
	if params.InPress, err = pairs.boolean("in_press"); err != nil {
		return nil, params, err
	}
	if params.OpenAccess, err = pairs.boolean("open_access"); err != nil {
		return nil, params, err
	}
	params.DateFrom, params.DateTo = pairs.value("date_from"), pairs.value("date_to")
	params.Doi, params.Pmid, params.Query = pairs.value("doi"), pairs.value("pmid"), pairs.value("q")
	if err = parseArticleSearch(pairs, &params); err != nil {
		return nil, params, err
	}
	if err = parseArticlePaging(pairs, &params); err != nil {
		return nil, params, err
	}
	return pairs.value("db"), params, nil
}

// parseArticleSearch validates the search mode before assigning an optional sort.
func parseArticleSearch(pairs queryPairs, params *query.ArticleListParams) *apiError {
	if mode := pairs.value("search_mode"); mode != nil {
		switch asciiLower(*mode) {
		case "simple", "advanced":
			params.SearchMode = domain.SearchMode(asciiLower(*mode))
		default:
			return badRequest("search_mode must be simple or advanced")
		}
	}
	if sort := pairs.value("sort"); sort != nil {
		params.Sort = sort
	}
	return nil
}

// parseArticleJournals preserves repeated nonempty filters and their validation order.
func parseArticleJournals(pairs queryPairs, params *query.ArticleListParams) *apiError {
	params.JournalId = []int64{}
	for _, value := range pairs.values("journal_id") {
		if strings.TrimSpace(value) == "" {
			continue
		}
		id, err := parseInteger("journal_id", value)
		if err != nil {
			return err
		}
		params.JournalId = append(params.JournalId, id)
	}
	return nil
}

// parseArticlePaging mutates pagination fields only after each preceding conversion succeeds.
func parseArticlePaging(pairs queryPairs, params *query.ArticleListParams) *apiError {
	limit, err := pairs.integer("limit")
	if err != nil {
		return err
	}
	if limit != nil {
		params.Limit = *limit
	}
	offset, err := pairs.integer("offset")
	if err != nil {
		return err
	}
	if offset != nil {
		params.Offset = *offset
	}
	params.Cursor = pairs.value("cursor")
	if params.IncludeTotal, err = pairs.boolean("include_total"); err != nil {
		return err
	}
	return nil
}

func asciiLower(value string) string {
	result := []byte(value)
	for index, character := range result {
		if character >= 'A' && character <= 'Z' {
			result[index] += 'a' - 'A'
		}
	}
	return string(result)
}

// parseWeeklyArticleQuery requires its identity fields before character and pagination checks.
func parseWeeklyArticleQuery(raw string) (query.WeeklyArticlePageParams, *apiError) {
	params := query.WeeklyArticlePageParams{Limit: 50}
	pairs, err := parseQueryPairs(raw)
	if err != nil {
		return params, err
	}
	database := pairs.value("db")
	if database == nil {
		return params, badRequest("db is required")
	}
	params.DbName = *database
	journal, err := pairs.integer("journal_id")
	if err != nil {
		return params, err
	}
	if journal == nil {
		return params, badRequest("journal_id is required")
	}
	if *journal <= 0 {
		return params, badRequest("journal_id must be greater than 0")
	}
	params.JournalId = *journal
	window := pairs.value("window_end")
	if window == nil {
		return params, badRequest("window_end is required")
	}
	params.WindowEnd, params.Query, params.Cursor = *window, pairs.value("q"), pairs.value("cursor")
	if failure := validateWeeklyQuery(params); failure != nil {
		return params, failure
	}
	limit, err := pairs.integer("limit")
	if err != nil {
		return params, err
	}
	if limit != nil {
		params.Limit = *limit
	}
	return params, nil
}

// validateWeeklyQuery retains database-first character checks before pagination conversion.
func validateWeeklyQuery(params query.WeeklyArticlePageParams) *apiError {
	if err := validateCharacters("db", params.DbName, 255); err != nil {
		return err
	}
	for _, field := range []struct {
		name  string
		value *string
	}{{"window_end", &params.WindowEnd}, {"q", params.Query}, {"cursor", params.Cursor}} {
		if field.value != nil {
			if err := validateCharacters(field.name, *field.value, 2048); err != nil {
				return err
			}
		}
	}
	return nil
}
