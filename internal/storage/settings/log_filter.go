package settings

import (
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

// validLogFilter follows the frozen EnvFilter directive state machine before regex validation.
func validLogFilter(value string) bool {
	for _, directive := range strings.Split(value, ",") {
		if directive != "" && !validLogDirective(directive) {
			return false
		}
	}
	return true
}

func validLogDirective(value string) bool {
	const (
		start = iota
		levelOrTarget
		span
		field
		fields
		target
		level
	)
	state, offset := start, 0
	slice := func(end int) (string, bool) {
		if offset > end || end > len(value) || !utf8.ValidString(value[offset:end]) {
			return "", false
		}
		return value[offset:end], true
	}
	for index, character := range strings.TrimSpace(value) {
		switch state {
		case start:
			if character == '[' {
				state = span
				offset = index + 1
			} else if character == '-' || character == ':' || character == '_' || isRustAlphabetic(character) || unicode.IsNumber(character) {
				state = levelOrTarget
				offset = index
			} else {
				return false
			}
		case levelOrTarget:
			if character == '=' {
				state = level
				offset = index + 1
			} else if character == '[' {
				state = span
				offset = index + 1
			}
		case span:
			if character == ']' {
				if _, ok := slice(index); !ok {
					return false
				}
				state = target
			} else if character == '{' {
				if _, ok := slice(index); !ok {
					return false
				}
				state = field
				offset = index + 1
			}
		case field:
			if character == '}' {
				candidate, ok := slice(index)
				if !ok || candidate == "" || !validLogField(candidate) {
					return false
				}
				state = fields
			}
		case fields:
			if character != ']' {
				return false
			}
			state = target
		case target:
			if character != '=' {
				return false
			}
			state = level
			offset = index + 1
		case level:
		}
	}
	switch state {
	case levelOrTarget, target:
		return true
	case level:
		remaining, ok := slice(len(value))
		return ok && (remaining == "" || validLogLevel(remaining))
	default:
		return false
	}
}

func validLogLevel(value string) bool {
	switch asciiLower(value) {
	case "off", "error", "warn", "info", "debug", "trace":
		return true
	}
	number, err := strconv.ParseUint(strings.TrimPrefix(value, "+"), 10, 64)
	return err == nil && number <= 5
}

func validLogField(value string) bool {
	parts := strings.Split(value, "=")
	if len(parts) < 2 {
		return true
	}
	pattern := parts[1]
	if pattern == "true" || pattern == "false" {
		return true
	}
	if _, err := strconv.ParseUint(strings.TrimPrefix(pattern, "+"), 10, 64); err == nil {
		return true
	}
	if _, err := strconv.ParseInt(pattern, 10, 64); err == nil {
		return true
	}
	numeric := pattern
	if strings.HasPrefix(numeric, "+") || strings.HasPrefix(numeric, "-") {
		numeric = numeric[1:]
	}
	switch asciiLower(numeric) {
	case "nan", "inf", "infinity":
		return true
	}
	if _, err := strconv.ParseFloat(pattern, 64); err == nil {
		return true
	}
	return validLogRegex(pattern)
}

func isRustAlphabetic(character rune) bool {
	return unicode.IsLetter(character) || unicode.Is(unicode.Nl, character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character)
}
