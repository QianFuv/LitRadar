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
	if !usesSimple || isAsciiProjection(value) {
		return value
	}
	var result strings.Builder
	for _, character := range norm.NFD.String(value) {
		writeProjectionRune(&result, character)
	}
	return result.String()
}

// isAsciiProjection admits only the original ASCII alphanumeric and whitespace fast path.
func isAsciiProjection(value string) bool {
	for _, character := range value {
		if !asciiAlphanumeric(character) && !asciiSpace(character) {
			return false
		}
	}
	return true
}

// isPrivateProjectionRune preserves the exact three private-use scalar ranges.
func isPrivateProjectionRune(character rune) bool {
	return character >= 0xe000 && character <= 0xf8ff || character >= 0xf0000 && character <= 0xffffd || character >= 0x100000 && character <= 0x10fffd
}

// writeProjectionRune drops marks before emitting preserved, lowercased or separator runes.
func writeProjectionRune(result *strings.Builder, character rune) {
	if unicode.IsMark(character) {
		return
	}
	isAlphanumeric := unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character)
	if !isAlphanumeric && !isPrivateProjectionRune(character) {
		result.WriteByte(' ')
	} else if character < 128 {
		result.WriteRune(character)
	} else {
		result.WriteRune(unicode.ToLower(character))
	}
}

// PrepareQuery normalizes advanced operands without changing operators, columns or malformed quote handling.
func PrepareQuery(value string, usesSimple bool, mode domain.SearchMode) string {
	if !usesSimple {
		return value
	}
	if mode != domain.SearchAdvanced {
		return prepareSimpleQuery(value)
	}
	return prepareAdvancedQuery(value)
}

// prepareSimpleQuery preserves nonblank input when projection produces only whitespace.
func prepareSimpleQuery(value string) string {
	normalized := PrepareText(value, true)
	if strings.TrimSpace(normalized) == "" && strings.TrimSpace(value) != "" {
		return value
	}
	return normalized
}

// advancedQuery retains rune position, column-set admission and output for one query.
type advancedQuery struct {
	characters  []rune
	position    int
	output      strings.Builder
	isColumnSet bool
}

// isAdvancedOperand preserves the original ASCII and unrestricted non-ASCII operand admission.
func isAdvancedOperand(character rune) bool {
	return asciiAlphanumeric(character) || character == '_' || character == 0x1a || character > 127
}

// isColumn looks past only ASCII whitespace while preserving boolean column-set state.
func (query *advancedQuery) isColumn() bool {
	if query.isColumnSet {
		return true
	}
	position := query.position
	for position < len(query.characters) && asciiSpace(query.characters[position]) {
		position++
	}
	return position < len(query.characters) && query.characters[position] == ':'
}

// quote emits the original FTS escaped operand spelling.
func (query *advancedQuery) quote(value string) {
	query.output.WriteByte('"')
	query.output.WriteString(strings.ReplaceAll(value, `"`, `""`))
	query.output.WriteByte('"')
}

// readQuotedOperand consumes doubled quotes and reports an unterminated operand.
func (query *advancedQuery) readQuotedOperand() (string, bool) {
	var operand strings.Builder
	for query.position < len(query.characters) {
		next := query.characters[query.position]
		query.position++
		if next == '"' {
			if query.position < len(query.characters) && query.characters[query.position] == '"' {
				query.position++
				operand.WriteByte('"')
			} else {
				return operand.String(), true
			}
		} else {
			operand.WriteRune(next)
		}
	}
	return operand.String(), false
}

// writeQuotedOperand preserves column spelling and normalizes only a completely closed operand.
func (query *advancedQuery) writeQuotedOperand() bool {
	start := query.position - 1
	operand, isClosed := query.readQuotedOperand()
	if !isClosed {
		return false
	}
	if query.isColumn() {
		query.output.WriteString(string(query.characters[start:query.position]))
	} else {
		query.quote(PrepareText(operand, true))
	}
	return true
}

// writeBareOperand preserves unchanged operands and columns before quoting normalized text.
func (query *advancedQuery) writeBareOperand() {
	start := query.position - 1
	for query.position < len(query.characters) && isAdvancedOperand(query.characters[query.position]) {
		query.position++
	}
	operand := string(query.characters[start:query.position])
	normalized := PrepareText(operand, true)
	if query.isColumn() || normalized == operand {
		query.output.WriteString(operand)
	} else {
		query.quote(normalized)
	}
}

// prepareAdvancedQuery retains ordinary syntax and returns the whole original input after a late open quote.
func prepareAdvancedQuery(value string) string {
	query := advancedQuery{characters: []rune(value)}
	for query.position < len(query.characters) {
		character := query.characters[query.position]
		query.position++
		if character == '"' {
			if !query.writeQuotedOperand() {
				return value
			}
		} else if isAdvancedOperand(character) {
			query.writeBareOperand()
		} else {
			if character == '{' {
				query.isColumnSet = true
			}
			if character == '}' {
				query.isColumnSet = false
			}
			query.output.WriteRune(character)
		}
	}
	return query.output.String()
}
