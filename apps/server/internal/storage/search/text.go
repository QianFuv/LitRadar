// Package search prepares byte-compatible FTS projections and query operands.
package search

import (
	"strings"
	"unicode"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/storage"
	"golang.org/x/text/unicode/norm"
)

// Tokenize implements Simple's byte classes with truncated characters clamped to the input.
// Only ASCII letters are folded; non-ASCII bytes retain their original spelling.
func Tokenize(value string) []string {
	tokens := []string{}
	for position := 0; position < len(value); {
		character := value[position]
		if character <= 32 || character == 127 {
			position++
			continue
		}
		start := position
		switch {
		case isASCIILetter(character):
			for position < len(value) && isASCIILetter(value[position]) {
				position++
			}
			tokens = append(tokens, strings.ToLower(value[start:position]))
			continue
		case character >= '0' && character <= '9':
			for position < len(value) && value[position] >= '0' && value[position] <= '9' {
				position++
			}
		case character >= 0xf0:
			position += 4
		case character >= 0xe0:
			position += 3
		case character >= 0xc0:
			position += 2
		default:
			position++
		}
		position = min(position, len(value))
		tokens = append(tokens, value[start:position])
	}
	return tokens
}

func isASCIILetter(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z'
}

// asciiSpace retains the legacy projection and column-lookahead whitespace set.
func asciiSpace(character byte) bool {
	return character == ' ' || character == '\t' || character == '\n' || character == '\r' || character == '\f'
}
func asciiAlphanumeric(character byte) bool {
	return character < 128 && (isASCIILetter(character) || character >= '0' && character <= '9')
}

// FoldText preserves the legacy search projection before byte tokenization or related-term extraction.
func FoldText(value string) string {
	if isASCIIProjection(value) {
		return value
	}
	decomposed := strings.TrimLeftFunc(norm.NFD.String(value), unicode.IsMark)
	return strings.Map(projectionRune, decomposed)
}

// isASCIIProjection admits only the original ASCII alphanumeric and whitespace fast path.
func isASCIIProjection(value string) bool {
	for _, character := range value {
		if character > 127 || !asciiAlphanumeric(byte(character)) && !asciiSpace(byte(character)) {
			return false
		}
	}
	return true
}

// isPrivateProjectionRune preserves the exact three private-use scalar ranges.
func isPrivateProjectionRune(character rune) bool {
	return character >= 0xe000 && character <= 0xf8ff || character >= 0xf0000 && character <= 0xffffd || character >= 0x100000 && character <= 0x10fffd
}

// projectionRune retains the original mark, separator and Unicode case rules.
func projectionRune(character rune) rune {
	if character < 128 {
		if asciiAlphanumeric(byte(character)) {
			return character
		}
		return ' '
	}
	if unicode.IsMark(character) {
		return -1
	}
	isAlphanumeric := unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character)
	if !isAlphanumeric && !isPrivateProjectionRune(character) {
		return ' '
	}
	return unicode.ToLower(character)
}

// PrepareText folds canonical text and joins byte tokens for the ASCII FTS tokenizer.
func PrepareText(value string) string { return strings.Join(Tokenize(FoldText(value)), " ") }

// Phrase quotes a simple operand, retaining nonblank input whose folded projection is blank.
func Phrase(value string) string {
	return `"` + strings.ReplaceAll(PrepareQuery(value, domain.SearchSimple), `"`, `""`) + `"`
}

// PrepareQuery folds and tokenizes operands while preserving advanced syntax and simple blank fallback.
func PrepareQuery(value string, mode domain.SearchMode) string {
	if mode != domain.SearchAdvanced {
		folded := FoldText(value)
		if strings.TrimSpace(folded) == "" && strings.TrimSpace(value) != "" {
			folded = value
		}
		return strings.Join(Tokenize(folded), " ")
	}
	return prepareAdvancedQuery(value)
}

// advancedQuery retains byte position, column-set admission and output for one query.
type advancedQuery struct {
	characters  string
	position    int
	output      strings.Builder
	isColumnSet bool
}

// isAdvancedOperand preserves the original ASCII and unrestricted non-ASCII operand admission.
func isAdvancedOperand(character byte) bool {
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
			operand.WriteByte(next)
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
		query.quote(PrepareText(operand))
	}
	return true
}

// writeBareOperand preserves operators and columns while quoting tokenized text.
func (query *advancedQuery) writeBareOperand() {
	start := query.position - 1
	for query.position < len(query.characters) && isAdvancedOperand(query.characters[query.position]) {
		query.position++
	}
	operand := string(query.characters[start:query.position])
	normalized := PrepareText(operand)
	isOperator := operand == "AND" || operand == "OR" || operand == "NOT"
	isNear := operand == "NEAR" && strings.HasPrefix(strings.TrimLeft(query.characters[query.position:], " \t\n\r\f\v"), "(")
	if query.isColumn() || isOperator || isNear || isNearDistance(query.characters, start) {
		query.output.WriteString(operand)
	} else {
		query.quote(normalized)
	}
}

// prepareAdvancedQuery retains ordinary syntax and returns the whole original input after a late open quote.
func prepareAdvancedQuery(value string) string {
	query := advancedQuery{characters: value}
	for query.position < len(query.characters) {
		character := query.characters[query.position]
		query.position++
		switch {
		case character == '"':
			if !query.writeQuotedOperand() {
				return value
			}
		case isAdvancedOperand(character):
			query.writeBareOperand()
		default:

			if character == '{' {
				query.isColumnSet = true
			}
			if character == '}' {
				query.isColumnSet = false
			}
			query.output.WriteByte(character)
		}
	}
	return query.output.String()
}

// isNearDistance leaves the numeric distance after a NEAR comma as syntax.
func isNearDistance(characters string, start int) bool {
	position := start - 1
	for position >= 0 && asciiSpace(characters[position]) {
		position--
	}
	return position >= 0 && characters[position] == ','
}
