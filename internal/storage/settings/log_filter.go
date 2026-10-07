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
	parser := logDirectiveParser{value: value}
	for index, character := range strings.TrimSpace(value) {
		if !parser.consume(index, character) {
			return false
		}
	}
	return parser.isComplete()
}

// logDirectiveState tracks the outer field lexer without normalizing its original byte offsets.
type logDirectiveState uint8

const (
	directiveStart logDirectiveState = iota
	directiveLevelOrTarget
	directiveSpan
	directiveField
	directiveFields
	directiveTarget
	directiveLevel
)

// logDirectiveParser preserves original-value slicing while traversing trimmed directive text.
type logDirectiveParser struct {
	value  string
	state  logDirectiveState
	offset int
}

// slice validates lexer offsets and the UTF-8 of the original source substring.
func (parser *logDirectiveParser) slice(end int) (string, bool) {
	if parser.offset > end || end > len(parser.value) || !utf8.ValidString(parser.value[parser.offset:end]) {
		return "", false
	}
	return parser.value[parser.offset:end], true
}

// consume advances one lexer state while retaining its original delimiter ordering.
func (parser *logDirectiveParser) consume(index int, character rune) bool {
	switch parser.state {
	case directiveStart:
		return parser.start(index, character)
	case directiveLevelOrTarget:
		parser.levelOrTarget(index, character)
	case directiveSpan:
		return parser.span(index, character)
	case directiveField:
		return parser.field(index, character)
	case directiveFields:
		if character != ']' {
			return false
		}
		parser.state = directiveTarget
	case directiveTarget:
		if character != '=' {
			return false
		}
		parser.state = directiveLevel
		parser.offset = index + 1
	case directiveLevel:
	}
	return true
}

// start accepts either an initial span bracket or a target/level name byte.
func (parser *logDirectiveParser) start(index int, character rune) bool {
	if character == '[' {
		parser.state = directiveSpan
		parser.offset = index + 1
		return true
	}
	if character == '-' || character == ':' || character == '_' || isAlphabetic(character) || unicode.IsNumber(character) {
		parser.state = directiveLevelOrTarget
		parser.offset = index
		return true
	}
	return false
}

// levelOrTarget keeps a target prefix open until a level separator or span bracket appears.
func (parser *logDirectiveParser) levelOrTarget(index int, character rune) {
	if character == '=' {
		parser.state = directiveLevel
		parser.offset = index + 1
	} else if character == '[' {
		parser.state = directiveSpan
		parser.offset = index + 1
	}
}

// span verifies its original substring before entering a field or target suffix.
func (parser *logDirectiveParser) span(index int, character rune) bool {
	if character == ']' {
		if _, ok := parser.slice(index); !ok {
			return false
		}
		parser.state = directiveTarget
	} else if character == '{' {
		if _, ok := parser.slice(index); !ok {
			return false
		}
		parser.state = directiveField
		parser.offset = index + 1
	}
	return true
}

// field validates the first brace-delimited field candidate without interpreting later separators.
func (parser *logDirectiveParser) field(index int, character rune) bool {
	if character == '}' {
		candidate, ok := parser.slice(index)
		if !ok || candidate == "" || !validLogField(candidate) {
			return false
		}
		parser.state = directiveFields
	}
	return true
}

// isComplete accepts target-only directives or a valid optional explicit level.
func (parser *logDirectiveParser) isComplete() bool {
	switch parser.state {
	case directiveLevelOrTarget, directiveTarget:
		return true
	case directiveLevel:
		remaining, ok := parser.slice(len(parser.value))
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

func isAlphabetic(character rune) bool {
	return unicode.IsLetter(character) || unicode.Is(unicode.Nl, character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character)
}
