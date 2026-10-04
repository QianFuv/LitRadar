package api

import (
	"errors"
	"fmt"
	"net/http"
	"strconv"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func extractTuplePathInteger(request *http.Request, name string, index int) (int64, *apiError) {
	value, failure := extractPathInteger(request, name)
	if failure != nil && strings.HasPrefix(failure.detail, "Invalid URL: Cannot parse") {
		failure.detail = fmt.Sprintf("Invalid URL: Cannot parse value at index %d with value `%s` to a `i64`", index, request.PathValue(name))
	}
	return value, failure
}

type queryKind byte

const (
	queryText queryKind = iota
	queryInteger
	queryBoolean
	queryUnsigned
)

type typedQuery map[string]any

func extractQuery(raw string, fields map[string]queryKind) (typedQuery, *apiError) {
	values := typedQuery{}
	for _, part := range strings.Split(raw, "&") {
		if part == "" {
			continue
		}
		name, value, _ := strings.Cut(part, "=")
		name, value = formDecode(name), formDecode(value)
		kind, known := fields[name]
		if !known {
			continue
		}
		if _, exists := values[name]; exists {
			return nil, queryRejection("duplicate field `" + name + "`")
		}
		switch kind {
		case queryText:
			values[name] = value
		case queryBoolean:
			if value != "true" && value != "false" {
				return nil, queryRejection(name + ": provided string was not `true` or `false`")
			}
			values[name] = value == "true"
		case queryInteger:
			number, err := strconv.ParseInt(value, 10, 64)
			if err != nil {
				detail := "invalid digit found in string"
				if value == "" {
					detail = "cannot parse integer from empty string"
				} else if errors.Is(err, strconv.ErrRange) {
					detail = "number too large to fit in target type"
					if strings.HasPrefix(value, "-") {
						detail = "number too small to fit in target type"
					}
				}
				return nil, queryRejection(name + ": " + detail)
			}
			values[name] = number
		case queryUnsigned:
			number, err := strconv.ParseUint(strings.TrimPrefix(value, "+"), 10, 64)
			if err != nil {
				detail := "invalid digit found in string"
				if value == "" {
					detail = "cannot parse integer from empty string"
				} else if errors.Is(err, strconv.ErrRange) {
					detail = "number too large to fit in target type"
				}
				return nil, queryRejection(name + ": " + detail)
			}
			values[name] = number
		}
	}
	return values, nil
}

func queryRejection(detail string) *apiError {
	return &apiError{status: 400, detail: "Failed to deserialize query string: " + detail, isPlain: true}
}

func formDecode(value string) string {
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

func extractPathInteger(request *http.Request, name string) (int64, *apiError) {
	value := request.PathValue(name)
	if !utf8.ValidString(value) {
		return 0, &apiError{status: 400, detail: "Invalid URL: Invalid UTF-8 in `" + name + "`", isPlain: true}
	}
	number, err := strconv.ParseInt(value, 10, 64)
	if err != nil {
		return 0, &apiError{status: 400, detail: "Invalid URL: Cannot parse `" + value + "` to a `i64`", isPlain: true}
	}
	return number, nil
}

func (values typedQuery) text(key string) *string {
	value, ok := values[key].(string)
	if !ok {
		return nil
	}
	return &value
}
func (values typedQuery) integer(key string) *int64 {
	value, ok := values[key].(int64)
	if !ok {
		return nil
	}
	return &value
}
func (values typedQuery) boolean(key string) *bool {
	value, ok := values[key].(bool)
	if !ok {
		return nil
	}
	return &value
}
func (values typedQuery) integerDefault(key string, fallback int64) int64 {
	if value := values.integer(key); value != nil {
		return *value
	}
	return fallback
}
func (values typedQuery) database() *string {
	if value := values.text("db"); value != nil {
		trimmed := strings.TrimSpace(*value)
		if trimmed != "" {
			return &trimmed
		}
	}
	return nil
}
