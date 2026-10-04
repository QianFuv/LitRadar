package mcp

import (
	"bytes"
	"encoding/json"
	"fmt"
	"slices"
	"strconv"
	"strings"
	"unicode/utf8"

	wire "github.com/QianFuv/LitRadar/internal/domain/api"
	sourcevalue "github.com/QianFuv/LitRadar/internal/domain/sources"
	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"github.com/QianFuv/LitRadar/internal/storage/query"
)

type toolSchema struct {
	Properties map[string]struct {
		Type  json.RawMessage
		AnyOf json.RawMessage
	} `json:"properties"`
	Required []string `json:"required"`
}

type toolInput struct {
	fields map[string]any
	err    error
}

func decodeInput(raw json.RawMessage, schema toolSchema) (*toolInput, error) {
	fields := make(map[string]any)
	if len(raw) != 0 {
		decoder := json.NewDecoder(bytes.NewReader(raw))
		decoder.UseNumber()
		if err := decoder.Decode(&fields); err != nil {
			return nil, err
		}
	}
	keys := make([]string, 0, len(fields))
	for key := range fields {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	for _, key := range keys {
		property, known := schema.Properties[key]
		if !known {
			continue
		}
		value := fields[key]
		isRequired := slices.Contains(schema.Required, key)
		if value == nil && !isRequired {
			continue
		}
		if len(property.AnyOf) != 0 {
			if _, ok := value.(string); ok {
				continue
			}
			if values, ok := value.([]any); ok {
				isStrings := true
				for _, item := range values {
					if _, ok := item.(string); !ok {
						isStrings = false
						break
					}
				}
				if isStrings {
					continue
				}
			}
			return nil, fmt.Errorf("data did not match any variant of untagged enum StringOrStrings")
		}
		var kind string
		if json.Unmarshal(property.Type, &kind) != nil {
			var kinds []string
			_ = json.Unmarshal(property.Type, &kinds)
			kind = kinds[0]
		}
		switch kind {
		case "string":
			if _, ok := value.(string); !ok {
				return nil, invalidType(value, "a string")
			}
		case "boolean":
			if _, ok := value.(bool); !ok {
				return nil, invalidType(value, "a boolean")
			}
		case "integer":
			number, ok := value.(json.Number)
			if !ok {
				return nil, invalidType(value, "i64")
			}
			parsedNumber, err := sourcevalue.ParseNumber(number)
			if err != nil {
				return nil, err
			}
			parsed, isInteger := parsedNumber.AsInt64()
			if !isInteger {
				if _, isUnsigned := parsedNumber.AsUint64(); isUnsigned {
					return nil, fmt.Errorf("invalid value: integer `%s`, expected i64", parsedNumber.String())
				}
				return nil, invalidType(value, "i64")
			}
			fields[key] = parsed
		case "array":
			values, ok := value.([]any)
			if !ok {
				return nil, invalidType(value, "a sequence")
			}
			for _, item := range values {
				if _, ok := item.(string); !ok {
					return nil, invalidType(item, "a string")
				}
			}
		}
	}
	for _, key := range schema.Required {
		if _, exists := fields[key]; !exists {
			return nil, fmt.Errorf("missing field `%s`", key)
		}
	}
	return &toolInput{fields: fields}, nil
}

func invalidType(value any, expected string) error {
	var actual string
	switch value := value.(type) {
	case nil:
		actual = "null"
	case string:
		actual = "string " + wire.DebugString(value)
	case bool:
		actual = fmt.Sprintf("boolean `%t`", value)
	case json.Number:
		kind := "integer"
		number, _ := sourcevalue.ParseNumber(value)
		if strings.ContainsAny(number.String(), ".eE") {
			kind = "floating point"
		}
		actual = fmt.Sprintf("%s `%s`", kind, number.String())
	case []any:
		actual = "sequence"
	default:
		actual = "map"
	}
	return fmt.Errorf("invalid type: %s, expected %s", actual, expected)
}

func (input *toolInput) fail(message string) {
	if input.err == nil {
		input.err = fmt.Errorf("%s", message)
	}
}

func (input *toolInput) requiredText(name, value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		input.fail(name + " must not be empty")
	} else if utf8.RuneCountInString(value) > 2048 {
		input.fail(name + " must be 1-2048 characters")
	}
	return value
}

func (input *toolInput) text(name string) *string {
	value, ok := input.fields[name].(string)
	if !ok {
		return nil
	}
	value = input.requiredText(name, value)
	return &value
}

func (input *toolInput) integer(name string) *int64 {
	value, ok := input.fields[name].(int64)
	if !ok {
		return nil
	}
	return &value
}

func (input *toolInput) nonnegative(name string) *int64 {
	value := input.integer(name)
	if value != nil && *value < 0 {
		input.fail(name + " must be greater than or equal to 0")
	}
	return value
}

func (input *toolInput) boolean(name string) *bool {
	value, ok := input.fields[name].(bool)
	if !ok {
		return nil
	}
	return &value
}

func (input *toolInput) strings(name string) []string {
	result := []string{}
	switch value := input.fields[name].(type) {
	case string:
		result = append(result, value)
	case []any:
		for _, item := range value {
			result = append(result, item.(string))
		}
	}
	return result
}

func (input *toolInput) positiveId(name, value string) int64 {
	value = input.requiredText(name, value)
	id, err := strconv.ParseInt(value, 10, 64)
	if err != nil || id <= 0 {
		input.fail(name + " must be a positive integer")
	}
	return id
}

func (input *toolInput) limit() int64 {
	if value := input.integer("limit"); value != nil {
		if *value < 1 || *value > 200 {
			input.fail("limit must be between 1 and 200")
		}
		return *value
	}
	return 50
}

func (input *toolInput) offset() int64 {
	if value := input.nonnegative("offset"); value != nil {
		return *value
	}
	return 0
}

func (input *toolInput) ratings() domain.JournalRatings {
	return domain.JournalRatings{UtdRating: input.strings("utd_rating"), AbsRating: input.strings("abs_rating"), FmsRating: input.strings("fms_rating"), FmscnRating: input.strings("fmscn_rating")}
}

func (input *toolInput) journals() (*string, query.JournalListParams) {
	database := input.text("db")
	params := query.JournalListParams{Area: input.text("area"), Ratings: input.ratings(), HasArticles: input.boolean("has_articles"), Year: input.nonnegative("year"), Sort: input.text("sort"), Limit: input.limit(), Offset: input.offset()}
	return database, params
}

func (input *toolInput) articles() (*string, query.ArticleListParams) {
	params := query.DefaultArticleListParams()
	params.JournalId = []int64{}
	ids := input.strings("journal_id")
	if len(ids) > 500 {
		input.fail("journal_id must contain at most 500 items")
	}
	for _, value := range ids {
		params.JournalId = append(params.JournalId, input.positiveId("journal_id", value))
	}
	params.Area = input.strings("area")
	if len(params.Area) > 500 {
		input.fail("area must contain at most 500 items")
	}
	for index, value := range params.Area {
		params.Area[index] = input.requiredText("area", value)
	}
	params.Ratings = input.ratings()
	params.IssueId, params.Year = input.nonnegative("issue_id"), input.nonnegative("year")
	params.InPress, params.OpenAccess = input.boolean("in_press"), input.boolean("open_access")
	params.DateFrom, params.DateTo = input.text("date_from"), input.text("date_to")
	params.Doi, params.Pmid, params.Query = input.text("doi"), input.text("pmid"), input.text("q")
	if mode := input.text("search_mode"); mode != nil {
		value := []byte(*mode)
		for index, character := range value {
			if character >= 'A' && character <= 'Z' {
				value[index] += 'a' - 'A'
			}
		}
		switch string(value) {
		case "simple", "advanced":
			params.SearchMode = domain.SearchMode(value)
		default:
			input.fail("search_mode must be simple or advanced")
		}
	}
	if sort := input.text("sort"); sort != nil {
		params.Sort = sort
	}
	params.Limit, params.Offset = input.limit(), input.offset()
	params.Cursor, params.IncludeTotal = input.text("cursor"), input.boolean("include_total")
	return input.text("db"), params
}
