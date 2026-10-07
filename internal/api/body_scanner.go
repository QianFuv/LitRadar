package api

import (
	"bytes"
	"fmt"
	"strconv"
	"strings"
	"unicode/utf8"
)

type bodyScanner struct {
	body       []byte
	position   int
	isIgnoring bool
}
type bodyFailure struct {
	detail        string
	isData        bool
	path          string
	needsPosition bool
}

func (failure *bodyFailure) Error() string { return failure.detail }
func (scanner *bodyScanner) failure(message string, lookahead bool) error {
	position := scanner.position
	if lookahead && position < len(scanner.body) {
		position++
	}
	prefix := scanner.body[:position]
	line := bytes.Count(prefix, []byte{'\n'}) + 1
	column := position - (bytes.LastIndexByte(prefix, '\n') + 1)
	return &bodyFailure{detail: fmt.Sprintf("%s at line %d column %d", message, line, column)}
}
func (scanner *bodyScanner) whitespace() {
	for scanner.position < len(scanner.body) && strings.ContainsRune(" \t\n\r", rune(scanner.body[scanner.position])) {
		scanner.position++
	}
}
func (scanner *bodyScanner) peek() byte {
	scanner.whitespace()
	if scanner.position == len(scanner.body) {
		return 0
	}
	return scanner.body[scanner.position]
}
func (scanner *bodyScanner) literal(value string) error {
	for _, character := range []byte(value) {
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing a value", false)
		}
		if scanner.body[scanner.position] != character {
			return scanner.failure("expected ident", true)
		}
		scanner.position++
	}
	return nil
}

// number scans each numeric production before applying typed range validation.
func (scanner *bodyScanner) number() error {
	start := scanner.position
	if err := scanner.integerDigits(); err != nil {
		return err
	}
	if err := scanner.fractionDigits(); err != nil {
		return err
	}
	didStop, err := scanner.exponent(start)
	if err != nil || didStop {
		return err
	}
	if _, err := strconv.ParseFloat(string(scanner.body[start:scanner.position]), 64); !scanner.isIgnoring && err != nil {
		return scanner.failure("number out of range", false)
	}
	return nil
}

// integerDigits consumes the optional sign and rejects missing or leading-zero digits.
func (scanner *bodyScanner) integerDigits() error {
	if scanner.body[scanner.position] == '-' {
		scanner.position++
	}
	if scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing a value", false)
	}
	if scanner.body[scanner.position] == '0' {
		scanner.position++
		if scanner.digit() {
			return scanner.failure("invalid number", true)
		}
	} else {
		if !scanner.digit() {
			return scanner.failure("invalid number", true)
		}
		for scanner.digit() {
			scanner.position++
		}
	}
	return nil
}

// fractionDigits requires at least one digit after a decimal point.
func (scanner *bodyScanner) fractionDigits() error {
	if scanner.position < len(scanner.body) && scanner.body[scanner.position] == '.' {
		scanner.position++
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing a value", false)
		}
		if !scanner.digit() {
			return scanner.failure("invalid number", true)
		}
		for scanner.digit() {
			scanner.position++
		}
	}
	return nil
}

// exponent scans an optional exponent and reports the original early overflow termination.
func (scanner *bodyScanner) exponent(start int) (bool, error) {
	if scanner.position < len(scanner.body) && (scanner.body[scanner.position] == 'e' || scanner.body[scanner.position] == 'E') {
		mantissa := scanner.body[start:scanner.position]
		scanner.position++
		positiveExponent := true
		if scanner.position < len(scanner.body) && (scanner.body[scanner.position] == '+' || scanner.body[scanner.position] == '-') {
			positiveExponent = scanner.body[scanner.position] != '-'
			scanner.position++
		}
		if scanner.position == len(scanner.body) {
			return false, scanner.failure("EOF while parsing a value", false)
		}
		if !scanner.digit() {
			return false, scanner.failure("invalid number", true)
		}
		return scanner.exponentDigits(mantissa, positiveExponent)
	}
	return false, nil
}

// exponentDigits preserves positive nonzero overflow rejection and ignored-value relaxation.
func (scanner *bodyScanner) exponentDigits(mantissa []byte, positiveExponent bool) (bool, error) {
	exponent := int64(0)
	nonzero := bytes.ContainsAny(mantissa, "123456789")
	for scanner.digit() {
		exponent = exponent*10 + int64(scanner.body[scanner.position]-'0')
		scanner.position++
		if exponent > 2147483647 {
			if !scanner.isIgnoring && nonzero && positiveExponent {
				return true, scanner.failure("number out of range", false)
			}
			for scanner.digit() {
				scanner.position++
			}
			return true, nil
		}
	}
	return false, nil
}
func (scanner *bodyScanner) digit() bool {
	return scanner.position < len(scanner.body) && scanner.body[scanner.position] >= '0' && scanner.body[scanner.position] <= '9'
}

func (scanner *bodyScanner) hexUnit() (uint64, error) {
	start := scanner.position
	for range 4 {
		if scanner.position == len(scanner.body) {
			return 0, scanner.failure("EOF while parsing a string", false)
		}
		scanner.position++
	}
	for _, character := range scanner.body[start:scanner.position] {
		if !strings.ContainsRune("0123456789abcdefABCDEF", rune(character)) {
			return 0, scanner.failure("invalid escape", false)
		}
	}
	value, _ := strconv.ParseUint(string(scanner.body[start:scanner.position]), 16, 16)
	return value, nil
}

// stringValue checks UTF-8 after consuming the closing quote.
func (scanner *bodyScanner) stringValue() error {
	scanner.position++
	start := scanner.position
	for scanner.position < len(scanner.body) {
		character := scanner.body[scanner.position]
		scanner.position++
		switch {
		case character == '"':
			if !scanner.isIgnoring && !utf8.Valid(scanner.body[start:scanner.position-1]) {
				return scanner.failure("invalid unicode code point", false)
			}
			return nil
		case character < 32:
			return scanner.failure("control character (\\u0000-\\u001F) found while parsing a string", false)
		case character == '\\':
			if err := scanner.stringEscape(); err != nil {
				return err
			}
		}
	}
	return scanner.failure("EOF while parsing a string", false)
}

// stringEscape validates one escape while retaining ignored Unicode relaxation.
func (scanner *bodyScanner) stringEscape() error {
	if scanner.position == len(scanner.body) {
		return scanner.failure("EOF while parsing a string", false)
	}
	escaped := scanner.body[scanner.position]
	scanner.position++
	if strings.ContainsRune(`"\/bfnrt`, rune(escaped)) {
		return nil
	}
	if escaped != 'u' {
		return scanner.failure("invalid escape", false)
	}
	value, err := scanner.hexUnit()
	if err != nil {
		return err
	}
	if scanner.isIgnoring {
		return nil
	}
	return scanner.surrogate(value)
}

// surrogate validates paired Unicode escapes at the original consumed-byte positions.
func (scanner *bodyScanner) surrogate(value uint64) error {
	if value >= 0xdc00 && value <= 0xdfff {
		return scanner.failure("lone leading surrogate in hex escape", false)
	}
	if value >= 0xd800 && value <= 0xdbff {
		if err := scanner.surrogatePrefix(); err != nil {
			return err
		}
		low, err := scanner.hexUnit()
		if err != nil {
			return err
		}
		if low < 0xdc00 || low > 0xdfff {
			return scanner.failure("lone leading surrogate in hex escape", false)
		}
	}
	return nil
}

// surrogatePrefix consumes the required second escape prefix before decoding its hex unit.
func (scanner *bodyScanner) surrogatePrefix() error {
	for _, expected := range []byte{'\\', 'u'} {
		if scanner.position == len(scanner.body) {
			return scanner.failure("EOF while parsing a string", false)
		}
		actual := scanner.body[scanner.position]
		scanner.position++
		if actual != expected {
			return scanner.failure("unexpected end of hex escape", false)
		}
	}
	return nil
}
