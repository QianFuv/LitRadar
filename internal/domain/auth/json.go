package auth

import (
	"bytes"
	"encoding/json"
	"io"
	"strconv"
	"strings"
	"unicode/utf8"
)

// ValidJson rejects the lossy substitutions and larger depth accepted by encoding/json.
func ValidJson(value string) bool {
	if !utf8.ValidString(value) {
		return false
	}
	for index := 0; index < len(value); index++ {
		if value[index] != '"' {
			continue
		}
		index++
		for index < len(value) && value[index] != '"' {
			if value[index] != '\\' {
				index++
				continue
			}
			index++
			if index >= len(value) {
				return false
			}
			if value[index] != 'u' {
				index++
				continue
			}
			if index+4 >= len(value) {
				return false
			}
			code, err := strconv.ParseUint(value[index+1:index+5], 16, 16)
			if err != nil {
				return false
			}
			index += 5
			if code >= 0xdc00 && code <= 0xdfff {
				return false
			}
			if code >= 0xd800 && code <= 0xdbff {
				if index+5 >= len(value) || value[index:index+2] != "\\u" {
					return false
				}
				low, err := strconv.ParseUint(value[index+2:index+6], 16, 16)
				if err != nil || low < 0xdc00 || low > 0xdfff {
					return false
				}
				index += 6
			}
		}
	}
	decoder := json.NewDecoder(strings.NewReader(value))
	decoder.UseNumber()
	depth := 0
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			return false
		}
		switch token := token.(type) {
		case json.Delim:
			if token == '{' || token == '[' {
				depth++
				if depth >= 128 {
					return false
				}
			} else {
				depth--
			}
		case json.Number:
			if _, err := strconv.ParseFloat(string(token), 64); err != nil {
				return false
			}
		}
	}
	return json.Valid([]byte(value))
}

// literalSeparators preserves serde's literal line separators without changing escaped backslashes.
func literalSeparators(value string) string {
	var result strings.Builder
	for index := 0; index < len(value); {
		if value[index] == '\\' && index+1 < len(value) {
			if index+6 <= len(value) && (value[index:index+6] == "\\u2028" || value[index:index+6] == "\\u2029") {
				if value[index+5] == '8' {
					result.WriteRune('\u2028')
				} else {
					result.WriteRune('\u2029')
				}
				index += 6
			} else {
				result.WriteString(value[index : index+2])
				index += 2
			}
		} else {
			result.WriteByte(value[index])
			index++
		}
	}
	return result.String()
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
