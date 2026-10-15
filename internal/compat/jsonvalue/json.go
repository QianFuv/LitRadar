// Package jsonvalue preserves persisted and wire JSON compatibility.
package jsonvalue

import (
	"bytes"
	"encoding/json"
	"strconv"
	"strings"
	"unicode/utf8"
)

// ValidJson rejects the lossy substitutions and larger depth accepted by encoding/json.
func ValidJson(value string) bool {
	if !utf8.ValidString(value) || !validJsonStringEscapes(value) {
		return false
	}
	return json.Valid([]byte(value)) && validJsonTokens(value)
}

// validJsonStringEscapes scans quoted strings without treating escaped quotes as boundaries.
func validJsonStringEscapes(value string) bool {
	for index := 0; index < len(value); index++ {
		if value[index] != '"' {
			continue
		}
		next, isValid := jsonStringEscapeEnd(value, index+1)
		if !isValid {
			return false
		}
		index = next
	}
	return true
}

// jsonStringEscapeEnd retains byte advancement while checking Unicode escape compatibility.
func jsonStringEscapeEnd(value string, index int) (int, bool) {
	for index < len(value) && value[index] != '"' {
		if value[index] != '\\' {
			index++
			continue
		}
		index++
		if index >= len(value) {
			return index, false
		}
		if value[index] != 'u' {
			index++
			continue
		}
		next, isValid := jsonUnicodeEscapeEnd(value, index)
		if !isValid {
			return index, false
		}
		index = next
	}
	return index, true
}

// jsonUnicodeEscapeEnd rejects isolated surrogates and consumes paired escapes together.
func jsonUnicodeEscapeEnd(value string, index int) (int, bool) {
	if index+4 >= len(value) {
		return index, false
	}
	code, err := strconv.ParseUint(value[index+1:index+5], 16, 16)
	if err != nil {
		return index, false
	}
	index += 5
	if code >= 0xdc00 && code <= 0xdfff {
		return index, false
	}
	if code >= 0xd800 && code <= 0xdbff {
		return jsonLowSurrogateEnd(value, index)
	}
	return index, true
}

// jsonLowSurrogateEnd requires an immediately adjacent low surrogate after a high surrogate.
func jsonLowSurrogateEnd(value string, index int) (int, bool) {
	if index+5 >= len(value) || value[index:index+2] != "\\u" {
		return index, false
	}
	low, err := strconv.ParseUint(value[index+2:index+6], 16, 16)
	if err != nil || low < 0xdc00 || low > 0xdfff {
		return index, false
	}
	return index + 6, true
}

// validJsonTokens scans syntactically valid JSON for shared depth and finite-number limits.
func validJsonTokens(value string) bool {
	depth := 0
	for index := 0; index < len(value); index++ {
		switch value[index] {
		case '"':
			for index++; value[index] != '"'; index++ {
				if value[index] == '\\' {
					index++
				}
			}
		case '{', '[':
			depth++
			if depth >= 128 {
				return false
			}
		case '}', ']':
			depth--
		default:
			if value[index] == '-' || value[index] >= '0' && value[index] <= '9' {
				start := index
				for index+1 < len(value) && strings.ContainsRune("0123456789.eE+-", rune(value[index+1])) {
					index++
				}
				if _, err := strconv.ParseFloat(value[start:index+1], 64); err != nil {
					return false
				}
			}
		}
	}
	return true
}

// literalSeparators preserves serde's literal line separators without changing escaped backslashes.
func literalSeparators(value string) string {
	if !strings.Contains(value, `\u2028`) && !strings.Contains(value, `\u2029`) {
		return value
	}
	var result strings.Builder
	result.Grow(len(value))
	for index := 0; index < len(value); {
		if value[index] != '\\' || index+1 >= len(value) {
			result.WriteByte(value[index])
			index++
			continue
		}
		index += writeLiteralSeparatorEscape(&result, value[index:])
	}
	return result.String()
}

// writeLiteralSeparatorEscape converts only exact separator escapes and preserves escape parity.
func writeLiteralSeparatorEscape(result *strings.Builder, value string) int {
	if len(value) >= 6 && (value[:6] == "\\u2028" || value[:6] == "\\u2029") {
		if value[5] == '8' {
			result.WriteRune('\u2028')
		} else {
			result.WriteRune('\u2029')
		}
		return 6
	}
	result.WriteString(value[:2])
	return 2
}

// EncodeJson preserves serde string escaping and deterministic map ordering.
func EncodeJson(value any) (string, error) {
	var buffer bytes.Buffer
	encoder := json.NewEncoder(&buffer)
	encoder.SetEscapeHTML(false)
	if err := encoder.Encode(value); err != nil {
		return "", err
	}
	return literalSeparators(strings.TrimSuffix(buffer.String(), "\n")), nil
}
