package settings

import (
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"
)

// logRegexParser validates the regex-syntax grammar reachable through EnvFilter's field lexer.
// Braced escapes and repetitions cannot reach this parser intact: the outer lexer closes at the first brace.
type logRegexParser struct {
	input    []rune
	position int
	isValid  bool
	names    map[string]bool
}
type logRegexFlags struct {
	isUnicode     bool
	isExtended    bool
	isInsensitive bool
}
type logRegexAtom struct {
	depth    int
	scalar   rune
	isScalar bool
	bytes    [256]bool
}

func validLogRegex(pattern string) bool {
	parser := logRegexParser{input: []rune(pattern), isValid: utf8.ValidString(pattern), names: map[string]bool{}}
	parser.expression(logRegexFlags{isUnicode: true}, false, 0)
	return parser.isValid && parser.position == len(parser.input)
}

func (parser *logRegexParser) skip(flags logRegexFlags) {
	if !flags.isExtended {
		return
	}
	for parser.position < len(parser.input) {
		character := parser.input[parser.position]
		if unicode.IsSpace(character) {
			parser.position++
			continue
		}
		if character == '#' {
			for parser.position < len(parser.input) && parser.input[parser.position] != '\n' {
				parser.position++
			}
			continue
		}
		break
	}
}
func (parser *logRegexParser) peek(character rune) bool {
	return parser.position < len(parser.input) && parser.input[parser.position] == character
}
func (parser *logRegexParser) take(character rune) bool {
	if parser.peek(character) {
		parser.position++
		return true
	}
	return false
}

func (parser *logRegexParser) expression(flags logRegexFlags, isGroup bool, nesting int) int {
	if nesting > 250 {
		parser.isValid = false
		return 0
	}
	terms := []int{}
	isLastFlags := false
	alternatives := []int{}
	concatenation := func() int {
		depth := 0
		for _, item := range terms {
			depth = max(depth, item)
		}
		if len(terms) > 1 {
			depth++
		}
		return depth
	}
	for parser.isValid {
		parser.skip(flags)
		if parser.position == len(parser.input) {
			if isGroup {
				parser.isValid = false
			}
			break
		}
		character := parser.input[parser.position]
		if character == ')' {
			if !isGroup {
				parser.isValid = false
			} else {
				parser.position++
			}
			break
		}
		if character == '|' {
			alternatives = append(alternatives, concatenation())
			terms = nil
			isLastFlags = false
			parser.position++
			continue
		}
		if character == '*' || character == '+' || character == '?' {
			if len(terms) == 0 || isLastFlags {
				parser.isValid = false
				break
			}
			parser.position++
			terms[len(terms)-1]++
			parser.take('?')
			continue
		}
		depth := 0
		isLastFlags = false
		switch character {
		case '{', '}':
			parser.isValid = false
		case '(':
			parser.position++
			parser.skip(flags)
			inner := flags
			if parser.take('?') {
				if parser.take('P') {
					if !parser.take('<') {
						parser.isValid = false
						break
					}
					parser.captureName()
					depth = parser.expression(inner, true, nesting+1) + 1
				} else if parser.take('<') {
					parser.captureName()
					depth = parser.expression(inner, true, nesting+1) + 1
				} else {
					seen := map[rune]bool{}
					isNegative := false
					lastNegative := false
					for parser.position < len(parser.input) && !parser.peek(':') && !parser.peek(')') {
						flag := parser.input[parser.position]
						parser.position++
						if seen[flag] || !strings.ContainsRune("imsURux-", flag) {
							parser.isValid = false
							break
						}
						seen[flag] = true
						if flag == '-' {
							isNegative = true
							lastNegative = true
							continue
						}
						lastNegative = false
						switch flag {
						case 'u':
							inner.isUnicode = !isNegative
						case 'x':
							inner.isExtended = !isNegative
						case 'i':
							inner.isInsensitive = !isNegative
						}
					}
					if lastNegative {
						parser.isValid = false
					}
					if parser.take(')') {
						if len(seen) == 0 {
							parser.isValid = false
						}
						flags = inner
						isLastFlags = true
					} else if parser.take(':') {
						depth = parser.expression(inner, true, nesting+1) + 1
					} else {
						parser.isValid = false
					}
				}
			} else {
				depth = parser.expression(inner, true, nesting+1) + 1
			}
		case '[':
			atom := parser.class(flags, nesting+1)
			depth = atom.depth
		case '\\':
			parser.position++
			parser.escape(flags, false)
		case '.':
			parser.position++
			if !flags.isUnicode {
				parser.isValid = false
			}
		default:
			parser.position++
		}
		terms = append(terms, depth)
	}
	depth := concatenation()
	if len(alternatives) > 0 {
		for _, alternative := range alternatives {
			depth = max(depth, alternative)
		}
		depth++
	}
	if depth > 250 {
		parser.isValid = false
	}
	return depth
}

func (parser *logRegexParser) captureName() {
	start := parser.position
	for parser.position < len(parser.input) && !parser.peek('>') {
		character := parser.input[parser.position]
		if parser.position == start {
			if character != '_' && !isRustAlphabetic(character) {
				parser.isValid = false
			}
		} else if character != '_' && character != '.' && character != '[' && character != ']' && !isRustAlphabetic(character) && !unicode.IsNumber(character) {
			parser.isValid = false
		}
		parser.position++
	}
	name := string(parser.input[start:parser.position])
	if name == "" || parser.names[name] || !parser.take('>') {
		parser.isValid = false
	}
	parser.names[name] = true
}

func literalAtom(character rune) logRegexAtom {
	result := logRegexAtom{scalar: character, isScalar: true}
	if character >= 0 && character < 256 {
		result.bytes[character] = true
	}
	return result
}
func (parser *logRegexParser) escape(flags logRegexFlags, isClass bool) logRegexAtom {
	if parser.position >= len(parser.input) {
		parser.isValid = false
		return logRegexAtom{}
	}
	character := parser.input[parser.position]
	parser.position++
	switch character {
	case 'a':
		return literalAtom(7)
	case 'f':
		return literalAtom(12)
	case 't':
		return literalAtom(9)
	case 'n':
		return literalAtom(10)
	case 'r':
		return literalAtom(13)
	case 'v':
		return literalAtom(11)
	case 'x', 'u', 'U':
		digits := 2
		if character == 'u' {
			digits = 4
		} else if character == 'U' {
			digits = 8
		}
		var hexadecimal strings.Builder
		for index := 0; index < digits; index++ {
			parser.skip(flags)
			if parser.position >= len(parser.input) || !strings.ContainsRune("0123456789abcdefABCDEF", parser.input[parser.position]) {
				parser.isValid = false
				return logRegexAtom{}
			}
			hexadecimal.WriteRune(parser.input[parser.position])
			parser.position++
		}
		parser.skip(flags)
		value, err := strconv.ParseUint(hexadecimal.String(), 16, 32)
		if err != nil || value > 0x10ffff || value >= 0xd800 && value <= 0xdfff || !flags.isUnicode && character == 'x' && value > 127 {
			parser.isValid = false
		}
		return literalAtom(rune(value))
	case 'd', 'D', 's', 'S', 'w', 'W':
		result := logRegexAtom{}
		for index := range result.bytes {
			value := rune(index)
			switch character {
			case 'd', 'D':
				result.bytes[index] = value >= '0' && value <= '9'
			case 's', 'S':
				result.bytes[index] = strings.ContainsRune(" \t\n\r\v\f", value)
			case 'w', 'W':
				result.bytes[index] = value == '_' || value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9'
			}
			if character == 'D' || character == 'S' || character == 'W' {
				result.bytes[index] = !result.bytes[index]
			}
		}
		if !flags.isUnicode && (character == 'D' || character == 'S' || character == 'W') {
			parser.isValid = false
		}
		return result
	case 'p', 'P':
		parser.skip(flags)
		if !flags.isUnicode || parser.position >= len(parser.input) || !strings.ContainsRune("LMNPSZClmnpszc", parser.input[parser.position]) {
			parser.isValid = false
		} else {
			parser.position++
		}
		return logRegexAtom{}
	case 'b', 'B', '<', '>':
		if isClass || flags.isUnicode {
			parser.isValid = false
		}
		return logRegexAtom{}
	case 'A', 'z':
		if isClass {
			parser.isValid = false
		}
		return logRegexAtom{}
	default:
		if character > 127 || unicode.IsLetter(character) || unicode.IsNumber(character) {
			parser.isValid = false
		}
		return literalAtom(character)
	}
}

func (parser *logRegexParser) class(flags logRegexFlags, nesting int) logRegexAtom {
	result := logRegexAtom{depth: 1}
	if nesting > 250 || !parser.take('[') {
		parser.isValid = false
		return result
	}
	parser.skip(flags)
	isNegated := parser.take('^')
	parser.skip(flags)
	terms := []logRegexAtom{}
	operands := []logRegexAtom{}
	operators := []rune{}
	union := func() logRegexAtom {
		merged := logRegexAtom{}
		for _, term := range terms {
			merged.depth = max(merged.depth, term.depth)
			for index, value := range term.bytes {
				merged.bytes[index] = merged.bytes[index] || value
			}
		}
		if len(terms) > 1 {
			merged.depth++
		}
		return merged
	}
	for parser.take('-') {
		terms = append(terms, literalAtom('-'))
		parser.skip(flags)
	}
	if len(terms) == 0 && parser.take(']') {
		terms = append(terms, literalAtom(']'))
	}
	for parser.isValid {
		parser.skip(flags)
		if parser.position >= len(parser.input) {
			parser.isValid = false
			break
		}
		if parser.take(']') {
			break
		}
		character := parser.input[parser.position]
		if parser.position+1 < len(parser.input) && parser.input[parser.position+1] == character && strings.ContainsRune("&-~", character) {
			operands = append(operands, union())
			operators = append(operators, character)
			terms = nil
			parser.position += 2
			continue
		}
		atom := parser.classAtom(flags, nesting)
		parser.skip(flags)
		if parser.hasRangeEnd(flags) {
			parser.position++
			parser.skip(flags)
			right := parser.classAtom(flags, nesting)
			if !atom.isScalar || !right.isScalar || atom.scalar > right.scalar || !flags.isUnicode && (atom.scalar > 127 || right.scalar > 127) {
				parser.isValid = false
			} else {
				start, end := atom.scalar, right.scalar
				for value := max(start, 0); value <= min(end, 255); value++ {
					atom.bytes[value] = true
				}
				atom.isScalar = false
			}
		}
		if !flags.isUnicode && atom.isScalar && atom.scalar > 127 {
			parser.isValid = false
		}
		terms = append(terms, atom)
	}
	operands = append(operands, union())
	result = operands[0]
	for index, operator := range operators {
		right := operands[index+1]
		result.depth = max(result.depth, right.depth) + 1
		for value := range result.bytes {
			switch operator {
			case '&':
				result.bytes[value] = result.bytes[value] && right.bytes[value]
			case '-':
				result.bytes[value] = result.bytes[value] && !right.bytes[value]
			case '~':
				result.bytes[value] = result.bytes[value] != right.bytes[value]
			}
		}
	}
	if flags.isInsensitive {
		for value := 'a'; value <= 'z'; value++ {
			if result.bytes[value] || result.bytes[value-32] {
				result.bytes[value] = true
				result.bytes[value-32] = true
			}
		}
	}
	if isNegated {
		for value := range result.bytes {
			result.bytes[value] = !result.bytes[value]
		}
	}
	if !flags.isUnicode {
		for value := 128; value < 256; value++ {
			if result.bytes[value] {
				parser.isValid = false
			}
		}
	}
	result.depth++
	result.isScalar = false
	if result.depth > 250 {
		parser.isValid = false
	}
	return result
}

func (parser *logRegexParser) hasRangeEnd(flags logRegexFlags) bool {
	if !parser.peek('-') {
		return false
	}
	position := parser.position + 1
	if flags.isExtended {
		isComment := false
		for index := position; index < len(parser.input); index++ {
			character := parser.input[index]
			if unicode.IsSpace(character) {
				continue
			}
			if !isComment && character == '#' {
				isComment = true
				continue
			}
			position = index
			break
		}
	}
	return position < len(parser.input) && parser.input[position] != '-' && parser.input[position] != ']'
}

func (parser *logRegexParser) classAtom(flags logRegexFlags, nesting int) logRegexAtom {
	if parser.position >= len(parser.input) {
		parser.isValid = false
		return logRegexAtom{}
	}
	if parser.peek('[') {
		remaining := string(parser.input[parser.position:])
		if strings.HasPrefix(remaining, "[:") {
			if end := strings.Index(remaining, ":]"); end >= 0 {
				name := remaining[2:end]
				isNegated := strings.HasPrefix(name, "^")
				name = strings.TrimPrefix(name, "^")
				if strings.Contains("|alnum|alpha|ascii|blank|cntrl|digit|graph|lower|print|punct|space|upper|word|xdigit|", "|"+name+"|") {
					if isNegated && !flags.isUnicode {
						parser.isValid = false
					}
					parser.position += utf8.RuneCountInString(remaining[:end+2])
					result := logRegexAtom{}
					for index := range result.bytes {
						character := rune(index)
						isDigit := character >= '0' && character <= '9'
						isLower := character >= 'a' && character <= 'z'
						isUpper := character >= 'A' && character <= 'Z'
						var matches bool
						switch name {
						case "alnum":
							matches = isDigit || isLower || isUpper
						case "alpha":
							matches = isLower || isUpper
						case "ascii":
							matches = index < 128
						case "blank":
							matches = character == ' ' || character == '\t'
						case "cntrl":
							matches = index < 32 || index == 127
						case "digit":
							matches = isDigit
						case "graph":
							matches = index >= 33 && index <= 126
						case "lower":
							matches = isLower
						case "print":
							matches = index >= 32 && index <= 126
						case "punct":
							matches = index >= 33 && index <= 126 && !isDigit && !isLower && !isUpper
						case "space":
							matches = strings.ContainsRune(" \t\n\r\v\f", character)
						case "upper":
							matches = isUpper
						case "word":
							matches = isDigit || isLower || isUpper || character == '_'
						case "xdigit":
							matches = isDigit || character >= 'a' && character <= 'f' || character >= 'A' && character <= 'F'
						}
						result.bytes[index] = matches != isNegated
					}
					return result
				}
			}
		}
		return parser.class(flags, nesting+1)
	}
	character := parser.input[parser.position]
	parser.position++
	if character == '\\' {
		return parser.escape(flags, true)
	}
	return literalAtom(character)
}
