// Package search preserves canonical records while preparing legacy-compatible FTS projections and operands.
package search

import (
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/internal/domain/storage"
	"golang.org/x/text/unicode/norm"
)

func asciiSpace(character rune) bool {
	return character == ' ' || character == '\t' || character == '\n' || character == '\r' || character == '\f'
}

func asciiAlphanumeric(character rune) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9'
}

// PrepareText folds non-ASCII case and accents only for the Simple projection, preserving source text.
func PrepareText(value string, usesSimple bool) string {
	if !usesSimple {
		return value
	}
	isAscii := true
	for _, character := range value {
		if !asciiAlphanumeric(character) && !asciiSpace(character) {
			isAscii = false
			break
		}
	}
	if isAscii {
		return value
	}
	var result strings.Builder
	for _, character := range norm.NFD.String(value) {
		if unicode.IsMark(character) {
			continue
		}
		isAlphanumeric := unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character)
		isPrivate := character >= 0xe000 && character <= 0xf8ff || character >= 0xf0000 && character <= 0xffffd || character >= 0x100000 && character <= 0x10fffd
		if !isAlphanumeric && !isPrivate {
			result.WriteByte(' ')
		} else if character < 128 {
			result.WriteRune(character)
		} else {
			result.WriteRune(unicode.ToLower(character))
		}
	}
	return result.String()
}

// PrepareQuery normalizes advanced operands without changing operators, columns or malformed quote handling.
func PrepareQuery(value string, usesSimple bool, mode domain.SearchMode) string {
	if !usesSimple {
		return value
	}
	if mode != domain.SearchAdvanced {
		normalized := PrepareText(value, true)
		if strings.TrimSpace(normalized) == "" && strings.TrimSpace(value) != "" {
			return value
		}
		return normalized
	}
	characters := []rune(value)
	var output strings.Builder
	isColumnSet := false
	isOperand := func(character rune) bool {
		return asciiAlphanumeric(character) || character == '_' || character == 0x1a || character > 127
	}
	isColumn := func(position int) bool {
		if isColumnSet {
			return true
		}
		for position < len(characters) && asciiSpace(characters[position]) {
			position++
		}
		return position < len(characters) && characters[position] == ':'
	}
	quote := func(value string) {
		output.WriteByte('"')
		output.WriteString(strings.ReplaceAll(value, `"`, `""`))
		output.WriteByte('"')
	}
	for position := 0; position < len(characters); {
		character := characters[position]
		position++
		if character == '"' {
			start := position - 1
			var operand strings.Builder
			isClosed := false
			for position < len(characters) {
				next := characters[position]
				position++
				if next == '"' {
					if position < len(characters) && characters[position] == '"' {
						position++
						operand.WriteByte('"')
					} else {
						isClosed = true
						break
					}
				} else {
					operand.WriteRune(next)
				}
			}
			if !isClosed {
				return value
			}
			if isColumn(position) {
				output.WriteString(string(characters[start:position]))
			} else {
				quote(PrepareText(operand.String(), true))
			}
		} else if isOperand(character) {
			start := position - 1
			for position < len(characters) && isOperand(characters[position]) {
				position++
			}
			operand := string(characters[start:position])
			normalized := PrepareText(operand, true)
			if isColumn(position) || normalized == operand {
				output.WriteString(operand)
			} else {
				quote(normalized)
			}
		} else {
			if character == '{' {
				isColumnSet = true
			}
			if character == '}' {
				isColumnSet = false
			}
			output.WriteRune(character)
		}
	}
	return output.String()
}
