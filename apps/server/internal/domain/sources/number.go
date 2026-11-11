// Package sources defines provider-neutral source values and validation rules.
package sources

import (
	"encoding/json"
	"errors"
	"strconv"
	"strings"
)

// ErrInvalidNumber rejects invalid or unrepresentable source JSON numbers.
var ErrInvalidNumber = errors.New("invalid source JSON number")

// Number retains the upstream JSON token until a consumer requests a typed number.
type Number struct{ value json.Number }

// ParseNumber validates a JSON number and its finite float64 representation.
func ParseNumber(value json.Number) (Number, error) {
	text := string(value)
	if text == "" || strings.TrimSpace(text) != text || text[0] != '-' && (text[0] < '0' || text[0] > '9') || !json.Valid([]byte(text)) {
		return Number{}, ErrInvalidNumber
	}
	if _, err := value.Float64(); err != nil {
		return Number{}, ErrInvalidNumber
	}
	return Number{value}, nil
}

// AsInt64 returns an integer token representable as int64.
func (number Number) AsInt64() (int64, bool) {
	value, err := number.value.Int64()
	return value, err == nil
}

// AsUint64 returns a nonnegative integer token representable as uint64.
func (number Number) AsUint64() (uint64, bool) {
	value, err := strconv.ParseUint(string(number.value), 10, 64)
	return value, err == nil
}

// AsFloat64 returns the standard correctly rounded float64 conversion.
func (number Number) AsFloat64() float64 { value, _ := number.value.Float64(); return value }

// String returns the original validated numeric token.
func (number Number) String() string { return string(number.value) }

// MarshalJSON encodes the validated numeric token without conversion through float64.
func (number Number) MarshalJSON() ([]byte, error) { return json.Marshal(number.value) }
