package sources

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"sort"
	"strconv"
	"strings"
	"unicode/utf8"
)

// Json encodes a decoded source value with the frozen serde_json representation.
// Object ordering and numeric formatting are part of persisted payload identity.
func Json(value any) ([]byte, error) {
	var output bytes.Buffer
	if err := appendJson(&output, value); err != nil {
		return nil, err
	}
	return output.Bytes(), nil
}

// appendJson dispatches structural values before the supported scalar representations.
func appendJson(output *bytes.Buffer, value any) error {
	switch item := value.(type) {
	case string:
		return appendJsonString(output, item)
	case []any:
		return appendJsonArray(output, item)
	case map[string]any:
		return appendJsonObject(output, item)
	default:
		return appendJsonScalar(output, value)
	}
}

// appendJsonString validates UTF-8 before emitting the original JSON escapes.
func appendJsonString(output *bytes.Buffer, item string) error {
	if !utf8.ValidString(item) {
		return fmt.Errorf("source JSON string is not UTF-8")
	}
	output.WriteByte('"')
	for _, character := range item {
		switch character {
		case '"', '\\':
			output.WriteByte('\\')
			output.WriteRune(character)
		case '\b':
			output.WriteString(`\b`)
		case '\f':
			output.WriteString(`\f`)
		case '\n':
			output.WriteString(`\n`)
		case '\r':
			output.WriteString(`\r`)
		case '\t':
			output.WriteString(`\t`)
		default:
			if character < 32 {
				fmt.Fprintf(output, "\\u%04x", character)
			} else {
				output.WriteRune(character)
			}
		}
	}
	output.WriteByte('"')
	return nil
}

// appendJsonArray preserves child order and the output prefix on a child failure.
func appendJsonArray(output *bytes.Buffer, item []any) error {
	output.WriteByte('[')
	for index, child := range item {
		if index > 0 {
			output.WriteByte(',')
		}
		if err := appendJson(output, child); err != nil {
			return err
		}
	}
	output.WriteByte(']')
	return nil
}

// appendJsonObject emits sorted keys and stops at the first key or child error.
func appendJsonObject(output *bytes.Buffer, item map[string]any) error {
	keys := make([]string, 0, len(item))
	for key := range item {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	output.WriteByte('{')
	for index, key := range keys {
		if index > 0 {
			output.WriteByte(',')
		}
		if err := appendJson(output, key); err != nil {
			return err
		}
		output.WriteByte(':')
		if err := appendJson(output, item[key]); err != nil {
			return err
		}
	}
	output.WriteByte('}')
	return nil
}

// appendJsonScalar emits supported scalar variants without changing numeric identity.
func appendJsonScalar(output *bytes.Buffer, value any) error {
	switch item := value.(type) {
	case nil:
		output.WriteString("null")
	case bool:
		output.WriteString(strconv.FormatBool(item))
	case json.Number:
		number, err := ParseNumber(item)
		if err != nil {
			return err
		}
		output.WriteString(number.String())
	case Number:
		output.WriteString(item.String())
	case float64:
		if math.IsNaN(item) || math.IsInf(item, 0) {
			return ErrInvalidNumber
		}
		output.WriteString(Number{kind: 'f', floating: item}.String())
	default:
		return appendJsonInteger(output, value)
	}
	return nil
}

// appendJsonInteger emits the supported native integer types and rejects other values.
func appendJsonInteger(output *bytes.Buffer, value any) error {
	switch item := value.(type) {
	case int:
		output.WriteString(strconv.Itoa(item))
	case int64:
		output.WriteString(strconv.FormatInt(item, 10))
	case uint64:
		output.WriteString(strconv.FormatUint(item, 10))
	default:
		return fmt.Errorf("unsupported source JSON value %T", value)
	}
	return nil
}

// String returns the serde_json decimal representation of this numeric variant.
func (number Number) String() string {
	switch number.kind {
	case 'u':
		return strconv.FormatUint(number.unsigned, 10)
	case 'i':
		return strconv.FormatInt(number.signed, 10)
	}
	text := strconv.FormatFloat(number.floating, 'e', -1, 64)
	parts := strings.Split(text, "e")
	exponent, _ := strconv.Atoi(parts[1])
	if exponent >= -5 && exponent <= 15 {
		text = strconv.FormatFloat(number.floating, 'f', -1, 64)
		if !strings.Contains(text, ".") {
			text += ".0"
		}
		return text
	}
	sign := "+"
	if exponent < 0 {
		sign = "-"
		exponent = -exponent
	}
	return parts[0] + "e" + sign + strconv.Itoa(exponent)
}
