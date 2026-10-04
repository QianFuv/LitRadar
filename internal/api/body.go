package api

import (
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"

	wire "github.com/QianFuv/LitRadar/internal/domain/api"
	"github.com/QianFuv/LitRadar/internal/domain/sources"
)

type bodyType struct {
	kind          string
	name          string
	fields        []bodyField
	element       *bodyType
	deniesUnknown bool
}
type bodyField struct {
	name       string
	value      bodyType
	hasDefault bool
	fallback   any
}

var stringBody = bodyType{kind: "string"}
var integerBody = bodyType{kind: "integer"}
var boolBody = bodyType{kind: "boolean"}
var articleBody = bodyType{kind: "article"}

func bodyFieldOf(name string, kind bodyType) bodyField { return bodyField{name: name, value: kind} }
func defaultBodyField(name string, kind bodyType, value any) bodyField {
	return bodyField{name, kind, true, value}
}
func structBody(name string, fields ...bodyField) bodyType {
	return bodyType{kind: "struct", name: name, fields: fields}
}
func listBody(element bodyType) bodyType { return bodyType{kind: "list", element: &element} }

func optionalBody(element bodyType) bodyType { return bodyType{kind: "optional", element: &element} }

var requestBodies = func() map[string]bodyType {
	favorite := structBody("FavoriteAdd", bodyFieldOf("article_id", articleBody), defaultBodyField("db_name", stringBody, ""), defaultBodyField("note", stringBody, ""))
	reference := structBody("FavoriteArticleRef", bodyFieldOf("article_id", articleBody), defaultBodyField("db_name", stringBody, ""))
	return map[string]bodyType{
		"ScheduledTaskCreate":        scheduledBody(false),
		"ScheduledTaskUpdate":        scheduledBody(true),
		"RuntimeSettingsUpdate":      runtimeSettingsBody(),
		"NotificationSettingsUpdate": notificationBody(),

		"AdminInviteCodeCreate":     {kind: "struct", name: "AdminInviteCodeCreate", deniesUnknown: true, fields: []bodyField{bodyFieldOf("expires_at", optionalBody(bodyType{kind: "float"})), bodyFieldOf("max_uses", optionalBody(integerBody))}},
		"LoginRequest":              structBody("LoginRequest", bodyFieldOf("username", stringBody), bodyFieldOf("password", stringBody)),
		"RegisterRequest":           structBody("RegisterRequest", bodyFieldOf("username", stringBody), bodyFieldOf("password", stringBody), bodyFieldOf("invite_code", stringBody)),
		"ChangePasswordRequest":     structBody("ChangePasswordRequest", bodyFieldOf("old_password", stringBody), bodyFieldOf("new_password", stringBody)),
		"TokenCreateRequest":        structBody("TokenCreateRequest", defaultBodyField("name", stringBody, ""), defaultBodyField("ttl", integerBody, int64(604800))),
		"FolderCreate":              structBody("FolderCreate", bodyFieldOf("name", stringBody), defaultBodyField("is_tracking", boolBody, false)),
		"FolderRename":              structBody("FolderRename", bodyFieldOf("name", stringBody)),
		"FavoriteAdd":               favorite,
		"FavoriteArticleRef":        reference,
		"FavoriteBatchCheckRequest": structBody("FavoriteBatchCheckRequest", bodyFieldOf("article_ids", listBody(articleBody)), defaultBodyField("db_name", stringBody, "")),
		"FavoriteBulkAdd":           structBody("FavoriteBulkAdd", bodyFieldOf("articles", listBody(favorite))),
		"FavoriteBulkRemove":        structBody("FavoriteBulkRemove", bodyFieldOf("articles", listBody(reference))),
		"FavoriteBulkMove":          structBody("FavoriteBulkMove", bodyFieldOf("target_folder_id", integerBody), bodyFieldOf("articles", listBody(reference))),
		"TrackingSetRequest":        structBody("TrackingSetRequest", bodyFieldOf("folder_id", integerBody)),
	}
}()

func extractBody(request *http.Request, kind bodyType, isOptional bool) (any, *apiError) {
	contentTypes, hasType := request.Header["Content-Type"]
	if isOptional && !hasType {
		return nil, nil
	}
	contentType := ""
	if len(contentTypes) > 0 {
		contentType = contentTypes[0]
	}
	if !isJsonContentType(contentType) {
		return nil, &apiError{status: 415, detail: "Expected request with `Content-Type: application/json`", isPlain: true}
	}
	const limit = 2 * 1024 * 1024
	body, err := io.ReadAll(io.LimitReader(request.Body, limit+1))
	if len(body) > limit {
		return nil, &apiError{status: 413, detail: "Failed to buffer the request body: length limit exceeded", isPlain: true}
	}
	if err != nil {
		return nil, &apiError{status: 400, detail: "Failed to buffer the request body: " + err.Error(), isPlain: true}
	}
	scanner := bodyScanner{body: body}
	value, err := scanner.typed(kind, "", 0)
	if err == nil && (scanner.peek() != 0 || scanner.position < len(scanner.body)) {
		err = scanner.failure("trailing characters", true)
	}
	if err == nil {
		return value, nil
	}
	failure := err.(*bodyFailure)
	status, prefix := 400, "Failed to parse the request body as JSON: "
	if failure.isData {
		status, prefix = 422, "Failed to deserialize the JSON body into the target type: "
	}
	if failure.path != "" {
		prefix += failure.path + ": "
	}
	return nil, &apiError{status: status, detail: prefix + failure.detail, isPlain: true}
}

func (scanner *bodyScanner) dataFailure(message string) error {
	failure := scanner.failure(message, false).(*bodyFailure)
	failure.isData = true
	return failure
}
func (scanner *bodyScanner) typed(kind bodyType, path string, depth int) (value any, err error) {
	defer func() {
		if err != nil {
			failure := err.(*bodyFailure)
			if failure.path == "" {
				failure.path = path
			}
		}
	}()
	token := scanner.peek()
	if token == 0 && scanner.position == len(scanner.body) {
		return nil, scanner.failure("EOF while parsing a value", false)
	}
	if kind.kind == "raw" {
		start := scanner.position
		if err := scanner.skip(); err != nil {
			return nil, err
		}
		return json.RawMessage(scanner.body[start:scanner.position]), nil
	}
	if kind.kind == "job" {
		return scanner.job(path, depth)
	}
	if kind.kind == "map" && token == '{' {
		return scanner.dictionary(*kind.element, path, depth)
	}
	if kind.kind == "optional" {
		if token == 'n' {
			return nil, scanner.literal("null")
		}
		return scanner.typed(*kind.element, path, depth)
	}
	if kind.kind == "struct" && (token == '{' || token == '[') {
		scanner.position++
		if depth+1 >= 128 {
			return nil, scanner.failure("recursion limit exceeded", false)
		}
		return scanner.structure(kind, path, depth+1, token == '[')
	}
	if kind.kind == "list" && token == '[' {
		scanner.position++
		if depth+1 >= 128 {
			return nil, scanner.failure("recursion limit exceeded", false)
		}
		values := []any{}
		for {
			if scanner.peek() == ']' {
				scanner.position++
				return values, nil
			}
			if scanner.position == len(scanner.body) {
				return nil, scanner.failure("EOF while parsing a list", false)
			}
			if len(values) > 0 {
				if err = scanner.separator(']'); err != nil {
					return nil, err
				}
				if scanner.position == len(scanner.body) {
					return nil, scanner.failure("EOF while parsing a value", false)
				}
			}
			item, err := scanner.typed(*kind.element, fmt.Sprintf("%s[%d]", path, len(values)), depth+1)
			if err != nil {
				return nil, err
			}
			values = append(values, item)
		}
	}
	start := scanner.position
	var actual string
	switch token {
	case '"':
		if err = scanner.stringValue(); err != nil {
			return nil, err
		}
		var text string
		_ = json.Unmarshal(scanner.body[start:scanner.position], &text)
		if kind.kind == "string" {
			return text, nil
		}
		if kind.kind == "article" {
			number, parseErr := strconv.ParseInt(text, 10, 64)
			if parseErr == nil {
				return number, nil
			}
			detail := "invalid digit found in string"
			if text == "" {
				detail = "cannot parse integer from empty string"
			} else if parseErr.(*strconv.NumError).Err == strconv.ErrRange {
				if strings.HasPrefix(text, "-") {
					detail = "number too small to fit in target type"
				} else {
					detail = "number too large to fit in target type"
				}
			}
			return nil, scanner.dataFailure(detail)
		}
		actual = "string " + wire.DebugString(text)
	case 't', 'f', 'n':
		literal := map[byte]string{'t': "true", 'f': "false", 'n': "null"}[token]
		if err = scanner.literal(literal); err != nil {
			return nil, err
		}
		if token != 'n' && kind.kind == "boolean" {
			return token == 't', nil
		}
		actual = "null"
		if token != 'n' {
			actual = "boolean `" + literal + "`"
		}
	case '{', '[':
		actual = map[byte]string{'{': "map", '[': "sequence"}[token]
	default:
		if token != '-' && (token < '0' || token > '9') {
			return nil, scanner.failure("expected value", true)
		}
		if err = scanner.number(); err != nil {
			return nil, err
		}
		number, parseErr := sources.ParseNumber(json.Number(scanner.body[start:scanner.position]))
		if parseErr != nil {
			return nil, scanner.failure("number out of range", false)
		}
		if kind.kind == "float" {
			return number.AsFloat64(), nil
		}
		if unsigned, ok := number.AsUint64(); ok && kind.kind == "unsigned" {
			return unsigned, nil
		}
		if signed, ok := number.AsInt64(); ok {
			if kind.kind == "unsigned" {
				expected := "u64"
				if kind.name != "" {
					expected = kind.name
				}
				return nil, scanner.dataFailure(fmt.Sprintf("invalid value: integer `%d`, expected %s", signed, expected))
			}
			if kind.kind == "integer" || kind.kind == "article" {
				return signed, nil
			}
			actual = "integer `" + number.String() + "`"
		} else if unsigned, ok := number.AsUint64(); ok {
			if kind.kind == "article" {
				return nil, scanner.dataFailure("out of range integral type conversion attempted")
			}
			if kind.kind == "integer" {
				return nil, scanner.dataFailure(fmt.Sprintf("invalid value: integer `%d`, expected i64", unsigned))
			}
			actual = "integer `" + number.String() + "`"
		} else {
			actual = "floating point `" + number.String() + "`"
		}
	}
	expected := map[string]string{"string": "a string", "integer": "i64", "unsigned": "u64", "float": "f64", "boolean": "a boolean", "article": "a SQLite integer or decimal string", "list": "a sequence", "map": "a map", "struct": "struct " + kind.name}[kind.kind]
	if kind.kind == "unsigned" && kind.name != "" {
		expected = kind.name
	}
	return nil, scanner.dataFailure("invalid type: " + actual + ", expected " + expected)
}

func (scanner *bodyScanner) separator(end byte) error {
	token := scanner.peek()
	if token == 0 && scanner.position == len(scanner.body) {
		kind := "a list"
		if end == '}' {
			kind = "an object"
		}
		return scanner.failure("EOF while parsing "+kind, false)
	}
	if token != ',' {
		return scanner.failure(fmt.Sprintf("expected `,` or `%c`", end), true)
	}
	scanner.position++
	if scanner.peek() == end {
		return scanner.failure("trailing comma", true)
	}
	return nil
}

func (scanner *bodyScanner) structure(kind bodyType, path string, depth int, isSequence bool) (any, error) {
	result := map[string]any{}
	end := byte('}')
	if isSequence {
		end = ']'
	}
	count := 0
	for scanner.peek() != end {
		if isSequence && count >= len(kind.fields) {
			if scanner.position == len(scanner.body) {
				return nil, scanner.failure("EOF while parsing a list", false)
			}
			if scanner.peek() == ',' {
				scanner.position++
				if scanner.peek() == ']' {
					return nil, scanner.failure("trailing comma", true)
				}
			}
			return nil, scanner.failure("trailing characters", true)
		}
		if isSequence && scanner.position == len(scanner.body) {
			return nil, scanner.failure("EOF while parsing a list", false)
		}
		if count > 0 {
			if err := scanner.separator(end); err != nil {
				return nil, err
			}
		}
		if isSequence && count >= len(kind.fields) {
			return nil, scanner.failure("trailing characters", true)
		}
		fieldIndex := count
		unknownKey := ""
		if !isSequence {
			if scanner.peek() == 0 && scanner.position == len(scanner.body) {
				if count > 0 {
					return nil, scanner.failure("EOF while parsing a value", false)
				}
				return nil, scanner.failure("EOF while parsing an object", false)
			}
			if scanner.peek() != '"' {
				return nil, scanner.failure("key must be a string", true)
			}
			start := scanner.position
			if err := scanner.stringValue(); err != nil {
				return nil, err
			}
			var key string
			_ = json.Unmarshal(scanner.body[start:scanner.position], &key)
			unknownKey = key
			fieldIndex = -1
			for index, field := range kind.fields {
				if field.name == key {
					fieldIndex = index
					break
				}
			}
			if _, exists := result[key]; exists {
				scanner.whitespace()
				if scanner.peek() == '}' {
					scanner.position++
				}
				return nil, scanner.dataFailure("duplicate field `" + key + "`")
			}
			if fieldIndex < 0 && kind.deniesUnknown {
				scanner.whitespace()
				if scanner.peek() == '}' {
					scanner.position++
				}
				names := make([]string, len(kind.fields))
				for index, field := range kind.fields {
					names[index] = "`" + field.name + "`"
				}
				expected := strings.Join(names, " or ")
				if len(names) > 2 {
					expected = "one of " + strings.Join(names, ", ")
				}
				failure := scanner.dataFailure("unknown field `" + key + "`, expected " + expected).(*bodyFailure)
				failure.path = key
				if path != "" {
					failure.path = path + "." + key
				}
				return nil, failure
			}
			if scanner.peek() == 0 && scanner.position == len(scanner.body) {
				return nil, scanner.failure("EOF while parsing an object", false)
			}
			if scanner.peek() != ':' {
				return nil, scanner.failure("expected `:`", true)
			}
			scanner.position++
		}
		if fieldIndex < 0 {
			if err := scanner.skip(); err != nil {
				failure := err.(*bodyFailure)
				failure.path = unknownKey
				if path != "" {
					failure.path = path + "." + unknownKey
				}
				return nil, err
			}
		} else {
			field := kind.fields[fieldIndex]
			childPath := field.name
			if isSequence {
				childPath = fmt.Sprintf("[%d]", fieldIndex)
			}
			if path != "" {
				if isSequence {
					childPath = path + childPath
				} else {
					childPath = path + "." + childPath
				}
			}
			value, err := scanner.typed(field.value, childPath, depth)
			if err != nil {
				failure := err.(*bodyFailure)
				if failure.needsPosition {
					token := scanner.peek()
					if token == end {
						scanner.position++
					} else if isSequence && token == ',' {
						scanner.position++
						scanner.peek()
					}
					positioned := scanner.failure(failure.detail, false).(*bodyFailure)
					failure.detail, failure.needsPosition = positioned.detail, false
				}
				return nil, err
			}
			result[field.name] = value
		}
		count++
	}
	scanner.position++
	for _, field := range kind.fields {
		if _, exists := result[field.name]; exists {
			continue
		}
		if field.hasDefault {
			result[field.name] = field.fallback
			continue
		}
		if !isSequence && field.value.kind == "optional" {
			result[field.name] = nil
			continue
		}
		if isSequence {
			return nil, scanner.dataFailure(fmt.Sprintf("invalid length %d, expected struct %s with %d elements", count, kind.name, len(kind.fields)))
		}
		return nil, scanner.dataFailure("missing field `" + field.name + "`")
	}
	return result, nil
}

// skip consumes ignored values iteratively, preserving serde's relaxed strings and numbers.
func (scanner *bodyScanner) skip() error {
	scanner.isIgnoring = true
	defer func() { scanner.isIgnoring = false }()
	stack := []byte{'v'}
	for len(stack) > 0 {
		action := stack[len(stack)-1]
		stack = stack[:len(stack)-1]
		token := scanner.peek()
		switch action {
		case 'v':
			switch token {
			case 0:
				if scanner.position < len(scanner.body) {
					return scanner.failure("expected value", true)
				}
				return scanner.failure("EOF while parsing a value", false)
			case '{':
				scanner.position++
				stack = append(stack, 'o')
			case '[':
				scanner.position++
				stack = append(stack, 'a')
			case '"':
				if err := scanner.stringValue(); err != nil {
					return err
				}
			case 't', 'f', 'n':
				if err := scanner.literal(map[byte]string{'t': "true", 'f': "false", 'n': "null"}[token]); err != nil {
					return err
				}
			default:
				if token != '-' && (token < '0' || token > '9') {
					return scanner.failure("expected value", true)
				}
				if err := scanner.number(); err != nil {
					return err
				}
			}
		case 'a', 'A':
			if token == ']' {
				scanner.position++
				continue
			}
			if action == 'A' {
				if err := scanner.separator(']'); err != nil {
					if scanner.peek() == ']' {
						return scanner.failure("expected value", true)
					}
					return err
				}
			}
			stack = append(stack, 'A', 'v')
		case 'o', 'O':
			if token == '}' {
				scanner.position++
				continue
			}
			if action == 'O' {
				if err := scanner.separator('}'); err != nil {
					if scanner.peek() == '}' {
						return scanner.failure("key must be a string", true)
					}
					return err
				}
			}
			if scanner.peek() == 0 && scanner.position == len(scanner.body) {
				return scanner.failure("EOF while parsing an object", false)
			}
			if scanner.peek() != '"' {
				return scanner.failure("key must be a string", true)
			}
			if err := scanner.stringValue(); err != nil {
				return err
			}
			if scanner.peek() == 0 && scanner.position == len(scanner.body) {
				return scanner.failure("EOF while parsing an object", false)
			}
			if scanner.peek() != ':' {
				return scanner.failure("expected `:`", true)
			}
			scanner.position++
			stack = append(stack, 'O', 'v')
		}
	}
	return nil
}
