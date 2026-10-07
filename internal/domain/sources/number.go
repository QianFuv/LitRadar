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
	if !validNumberToken(text) {
		return Number{}, ErrInvalidNumber
	}
	isNegative := text[0] == '-'
	if isNegative {
		text = text[1:]
	}
	significand, exponent, index, hasIntegerOverflow := scanIntegerSignificand(text)
	if index == len(text) && !hasIntegerOverflow {
		return integralNumber(significand, isNegative), nil
	}
	significand, exponent, index = scanFractionSignificand(text, significand, exponent, index)
	exponent, isZero, err := scanExplicitExponent(text, significand, exponent, index)
	if err != nil {
		return Number{}, err
	}
	if isZero {
		return floatingNumber(0, isNegative)
	}
	return scaleSignificand(significand, exponent, isNegative)
}

// validNumberToken requires the exact JSON numeric grammar without surrounding whitespace.
func validNumberToken(text string) bool {
	return !(text == "" || text != strings.TrimSpace(text) || text[0] != '-' && (text[0] < '0' || text[0] > '9') || !json.Valid([]byte(text)))
}

// integralNumber preserves unsigned positives, signed negatives and floating negative zero.
func integralNumber(significand uint64, isNegative bool) Number {
	if !isNegative {
		return Number{kind: 'u', unsigned: significand}
	}
	if significand > 0 && significand <= 1<<63 {
		return Number{kind: 'i', signed: -int64(significand)}
	}
	return Number{kind: 'f', floating: -float64(significand)}
}

// scanIntegerSignificand counts every discarded integer digit in the decimal exponent.
func scanIntegerSignificand(text string) (uint64, int64, int, bool) {
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
	return significand, exponent, index, hasIntegerOverflow
}

// scanFractionSignificand retains fractional digits only until the first overflow.
func scanFractionSignificand(text string, significand uint64, exponent int64, index int) (uint64, int64, int) {
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
	return significand, exponent, index
}

// scanExplicitExponent saturates ordinary exponents and preserves overflow zero admission.
func scanExplicitExponent(text string, significand uint64, exponent int64, index int) (int64, bool, error) {
	if index == len(text) {
		return exponent, false, nil
	}
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
				return 0, false, ErrInvalidNumber
			}
			return 0, true, nil
		}
		explicit = explicit*10 + digit
	}
	if isNegativeExponent {
		exponent = max(math.MinInt32, exponent-explicit)
	} else {
		exponent = min(math.MaxInt32, exponent+explicit)
	}
	return exponent, false, nil
}

// scaleSignificand applies decimal powers in the original order before sign conversion.
func scaleSignificand(significand uint64, exponent int64, isNegative bool) (Number, error) {
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
