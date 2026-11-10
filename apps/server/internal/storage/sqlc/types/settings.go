// Package types supplies storage validators shared with sqlc-generated queries.
package types

import (
	"errors"
	"unicode/utf8"
)

// SettingText validates runtime setting text without SQLite coercion.
type SettingText string

// Scan accepts valid native SQLite text.
func (value *SettingText) Scan(source any) error {
	decoded, ok := source.(string)
	if !ok || !utf8.ValidString(decoded) {
		return errors.New("invalid runtime setting text column")
	}
	*value = SettingText(decoded)
	return nil
}

// SettingReal validates runtime setting timestamps without SQLite coercion.
type SettingReal float64

// Scan accepts native SQLite numeric values.
func (value *SettingReal) Scan(source any) error {
	switch decoded := source.(type) {
	case float64:
		*value = SettingReal(decoded)
	case int64:
		*value = SettingReal(decoded)
	default:
		return errors.New("invalid runtime setting real column")
	}
	return nil
}
