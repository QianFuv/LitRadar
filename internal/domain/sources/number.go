// Package sources defines provider-neutral source values and compatibility rules.
package sources

import (
	"encoding/json"
	"errors"
	"math"
	"strconv"
	"strings"
)

// ErrInvalidNumber rejects numbers outside the frozen serde_json representation.
var ErrInvalidNumber = errors.New("invalid source JSON number")

// Number retains serde's integer/float distinction and non-roundtrip decimal conversion.
type Number struct {
	kind     byte
	unsigned uint64
	signed   int64
	floating float64
}

var decimalPowers = func() [309]float64 {
	var values [309]float64
	for index := range values {
		values[index], _ = strconv.ParseFloat("1e"+strconv.Itoa(index), 64)
	}
	return values
}()

// ParseNumber follows serde_json 1.0.150 without float_roundtrip or arbitrary_precision.
func ParseNumber(value json.Number) (Number, error) {
	text := string(value)
	if text == "" || text != strings.TrimSpace(text) || text[0] != '-' && (text[0] < '0' || text[0] > '9') || !json.Valid([]byte(text)) {
		return Number{}, ErrInvalidNumber
	}
	isNegative := text[0] == '-'
	if isNegative {
		text = text[1:]
	}
	var significand uint64
	exponent, index := int64(0), 0
	hasIntegerOverflow := false
	for index < len(text) && text[index] >= '0' && text[index] <= '9' {
		digit := uint64(text[index] - '0')
		if !hasIntegerOverflow && significand <= (math.MaxUint64-digit)/10 {
			significand = significand*10 + digit
		} else {
			hasIntegerOverflow = true
			exponent++
		}
		index++
	}
	if index == len(text) && !hasIntegerOverflow {
		if !isNegative {
			return Number{kind: 'u', unsigned: significand}, nil
		}
		if significand > 0 && significand <= 1<<63 {
			return Number{kind: 'i', signed: -int64(significand)}, nil
		}
		return Number{kind: 'f', floating: -float64(significand)}, nil
	}
	if index < len(text) && text[index] == '.' {
		index++
		hasFractionOverflow := false
		for index < len(text) && text[index] >= '0' && text[index] <= '9' {
			digit := uint64(text[index] - '0')
			if !hasFractionOverflow && significand <= (math.MaxUint64-digit)/10 {
				significand = significand*10 + digit
				exponent--
			} else {
				hasFractionOverflow = true
			}
			index++
		}
	}
	if index < len(text) {
		index++
		isNegativeExponent := false
		if text[index] == '+' || text[index] == '-' {
			isNegativeExponent = text[index] == '-'
			index++
		}
		var explicit int64
		for ; index < len(text); index++ {
			digit := int64(text[index] - '0')
			if explicit > (math.MaxInt32-digit)/10 {
				if significand != 0 && !isNegativeExponent {
					return Number{}, ErrInvalidNumber
				}
				return floatingNumber(0, isNegative)
			}
			explicit = explicit*10 + digit
		}
		if isNegativeExponent {
			exponent = max(math.MinInt32, exponent-explicit)
		} else {
			exponent = min(math.MaxInt32, exponent+explicit)
		}
	}
	floating := float64(significand)
	for {
		if exponent >= -308 && exponent <= 308 {
			if exponent >= 0 {
				floating *= decimalPowers[exponent]
			} else {
				floating /= decimalPowers[-exponent]
			}
			break
		}
		if floating == 0 {
			break
		}
		if exponent >= 0 {
			return Number{}, ErrInvalidNumber
		}
		floating /= 1e308
		exponent += 308
	}
	return floatingNumber(floating, isNegative)
}

func floatingNumber(value float64, isNegative bool) (Number, error) {
	if math.IsInf(value, 0) {
		return Number{}, ErrInvalidNumber
	}
	if isNegative {
		value = -value
	}
	return Number{kind: 'f', floating: value}, nil
}

// AsInt64 accepts only representable integral variants, never integral floats.
func (number Number) AsInt64() (int64, bool) {
	if number.kind == 'i' {
		return number.signed, true
	}
	if number.kind == 'u' && number.unsigned <= math.MaxInt64 {
		return int64(number.unsigned), true
	}
	return 0, false
}

// AsUint64 accepts only the unsigned integral representation.
func (number Number) AsUint64() (uint64, bool) { return number.unsigned, number.kind == 'u' }

// AsFloat64 converts integer variants or returns the already-rounded decimal value.
func (number Number) AsFloat64() float64 {
	switch number.kind {
	case 'u':
		return float64(number.unsigned)
	case 'i':
		return float64(number.signed)
	default:
		return number.floating
	}
}
