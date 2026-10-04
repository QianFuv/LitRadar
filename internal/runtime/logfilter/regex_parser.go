package logfilter

import (
	"errors"
	"strconv"
	"strings"
	"unicode"

	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

type regexFlags struct{ isUnicode, isInsensitive, isExtended, isMultiline, isDotAll, isUngreedy, isCrlf bool }
type regexParser struct {
	input    []rune
	position int
	err      error
}

func compileFieldRegex(pattern string) (*fieldRegex, error) {
	if _, err := settings.Normalize("log_filter", "[{value="+pattern+"}]"); err != nil {
		return nil, errors.New("invalid field regex")
	}
	parser := regexParser{input: []rune(pattern)}
	node := parser.expression(regexFlags{isUnicode: true}, false)
	if parser.err != nil || parser.position != len(parser.input) {
		return nil, errors.New("invalid field regex")
	}
	expression := &fieldRegex{}
	end := expression.emit(instruction{kind: 'm'})
	expression.start = expression.compile(node, end)
	return expression, nil
}

func (parser *regexParser) take(character rune) bool {
	if parser.position < len(parser.input) && parser.input[parser.position] == character {
		parser.position++
		return true
	}
	return false
}
func (parser *regexParser) skip(flags regexFlags) {
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
func (parser *regexParser) fail() { parser.err = errors.New("invalid field regex") }

func (parser *regexParser) expression(flags regexFlags, isGroup bool) *regexNode {
	sequence := &regexNode{kind: 'q'}
	alternatives := []*regexNode{}
	for parser.err == nil {
		parser.skip(flags)
		if parser.position == len(parser.input) {
			if isGroup {
				parser.fail()
			}
			break
		}
		if parser.take(')') {
			if !isGroup {
				parser.fail()
			}
			break
		}
		if parser.take('|') {
			alternatives = append(alternatives, sequence)
			sequence = &regexNode{kind: 'q'}
			continue
		}
		character := parser.input[parser.position]
		parser.position++
		if strings.ContainsRune("*+?", character) {
			if len(sequence.children) == 0 {
				parser.fail()
				break
			}
			isLazy := parser.take('?') != flags.isUngreedy
			last := len(sequence.children) - 1
			sequence.children[last] = &regexNode{kind: byte(character), children: []*regexNode{sequence.children[last]}, isLazy: isLazy}
			continue
		}
		var node *regexNode
		switch character {
		case '(':
			inner := flags
			parser.skip(flags)
			if parser.take('?') {
				isNamed := parser.take('<')
				if !isNamed && parser.take('P') {
					isNamed = parser.take('<')
					if !isNamed {
						parser.fail()
					}
				}
				if isNamed {
					for parser.position < len(parser.input) && !parser.take('>') {
						parser.position++
					}
				} else {
					isEnabled := true
					for parser.position < len(parser.input) && parser.input[parser.position] != ':' && parser.input[parser.position] != ')' {
						flag := parser.input[parser.position]
						parser.position++
						switch flag {
						case '-':
							isEnabled = false
						case 'i':
							inner.isInsensitive = isEnabled
						case 'm':
							inner.isMultiline = isEnabled
						case 's':
							inner.isDotAll = isEnabled
						case 'U':
							inner.isUngreedy = isEnabled
						case 'R':
							inner.isCrlf = isEnabled
						case 'u':
							inner.isUnicode = isEnabled
						case 'x':
							inner.isExtended = isEnabled
						default:
							parser.fail()
						}
					}
					if parser.take(')') {
						flags = inner
						continue
					}
					if !parser.take(':') {
						parser.fail()
					}
				}
			}
			node = parser.expression(inner, true)
		case '[':
			parser.position--
			node = &regexNode{kind: 'c', set: parser.class(flags)}
		case '\\':
			node = parser.escape(flags, false)
		case '.':
			set := runeSet{0, 0xd7ff, 0xe000, 0x10ffff}
			if !flags.isDotAll {
				excluded := runeSet{'\n', '\n'}
				if flags.isCrlf {
					excluded = append(excluded, '\r', '\r')
				}
				set = intersectSet(set, complementSet(normalizeSet(excluded), true))
			}
			node = &regexNode{kind: 'c', set: set}
		case '^', '$':
			assertion := "start"
			if character == '$' {
				assertion = "end"
			}
			if flags.isMultiline {
				assertion = "line_" + assertion
				if flags.isCrlf {
					assertion = strings.Replace(assertion, "line_", "crlf_", 1)
				}
			}
			node = &regexNode{kind: 'a', assertion: assertion}
		default:
			node = literalRegex(character, flags)
		}
		sequence.children = append(sequence.children, node)
	}
	if len(alternatives) == 0 {
		return sequence
	}
	return &regexNode{kind: '|', children: append(alternatives, sequence)}
}

func literalRegex(character rune, flags regexFlags) *regexNode {
	set := runeSet{character, character}
	if flags.isInsensitive {
		set = foldSet(set, flags.isUnicode)
	}
	return &regexNode{kind: 'c', set: set}
}

func (parser *regexParser) escape(flags regexFlags, isClass bool) *regexNode {
	if parser.position == len(parser.input) {
		parser.fail()
		return &regexNode{kind: 'q'}
	}
	character := parser.input[parser.position]
	parser.position++
	if decoded, exists := map[rune]rune{'a': 7, 'f': 12, 't': 9, 'n': 10, 'r': 13, 'v': 11}[character]; exists {
		return literalRegex(decoded, flags)
	}
	if assertion, exists := map[rune]string{'A': "start", 'z': "end", 'b': "boundary", 'B': "not_boundary", '<': "word_start", '>': "word_end"}[character]; exists {
		if isClass {
			parser.fail()
		}
		return &regexNode{kind: 'a', assertion: assertion}
	}
	switch character {
	case 'x', 'u', 'U':
		digits := 2
		if character == 'u' {
			digits = 4
		} else if character == 'U' {
			digits = 8
		}
		var text strings.Builder
		for range digits {
			parser.skip(flags)
			if parser.position == len(parser.input) {
				parser.fail()
				break
			}
			text.WriteRune(parser.input[parser.position])
			parser.position++
		}
		value, err := strconv.ParseUint(text.String(), 16, 32)
		if err != nil {
			parser.fail()
		}
		return literalRegex(rune(value), flags)
	case 'd', 'D', 's', 'S', 'w', 'W', 'p', 'P':
		key := strings.ToLower(string(character))
		isNegated := character >= 'A' && character <= 'Z'
		if character == 'p' || character == 'P' {
			parser.skip(flags)
			if parser.position == len(parser.input) {
				parser.fail()
				return &regexNode{kind: 'q'}
			}
			key = strings.ToUpper(string(parser.input[parser.position]))
			parser.position++
		}
		set := unicodeTables.Tables[key]
		if !flags.isUnicode {
			set = asciiClass(map[string]string{"d": "digit", "s": "space", "w": "word"}[key])
		}
		if flags.isInsensitive {
			set = foldSet(set, flags.isUnicode)
		}
		if isNegated {
			set = complementSet(set, flags.isUnicode)
		}
		return &regexNode{kind: 'c', set: set}
	default:
		return literalRegex(character, flags)
	}
}

func (parser *regexParser) class(flags regexFlags) runeSet {
	if !parser.take('[') {
		parser.fail()
		return nil
	}
	parser.skip(flags)
	isNegated := parser.take('^')
	parser.skip(flags)
	terms := runeSet{}
	operands := []runeSet{}
	operators := []rune{}
	for parser.take('-') {
		terms = append(terms, '-', '-')
		parser.skip(flags)
	}
	if len(terms) == 0 && parser.take(']') {
		terms = append(terms, ']', ']')
	}
	for parser.err == nil {
		parser.skip(flags)
		if parser.position == len(parser.input) {
			parser.fail()
			break
		}
		if parser.take(']') {
			break
		}
		character := parser.input[parser.position]
		if parser.position+1 < len(parser.input) && parser.input[parser.position+1] == character && strings.ContainsRune("&-~", character) {
			operands = append(operands, normalizeSet(terms))
			terms = nil
			operators = append(operators, character)
			parser.position += 2
			continue
		}
		set, isScalar, scalar := parser.classAtom(flags)
		parser.skip(flags)
		if parser.position+1 < len(parser.input) && parser.input[parser.position] == '-' && parser.input[parser.position+1] != '-' {
			saved := parser.position
			parser.position++
			parser.skip(flags)
			if parser.position < len(parser.input) && parser.input[parser.position] != ']' {
				_, isLastScalar, last := parser.classAtom(flags)
				if !isScalar || !isLastScalar || last < scalar {
					parser.fail()
				}
				set = runeSet{scalar, last}
			} else {
				parser.position = saved
			}
		}
		terms = append(terms, set...)
	}
	operands = append(operands, normalizeSet(terms))
	if flags.isInsensitive {
		for position := range operands {
			operands[position] = foldSet(operands[position], flags.isUnicode)
		}
	}
	result := operands[0]
	for position, operator := range operators {
		right := operands[position+1]
		switch operator {
		case '&':
			result = intersectSet(result, right)
		case '-':
			result = intersectSet(result, complementSet(right, flags.isUnicode))
		case '~':
			result = unionSet(intersectSet(result, complementSet(right, flags.isUnicode)), intersectSet(right, complementSet(result, flags.isUnicode)))
		}
	}
	if flags.isInsensitive {
		result = foldSet(result, flags.isUnicode)
	}
	if isNegated {
		result = complementSet(result, flags.isUnicode)
	}
	return result
}

func (parser *regexParser) classAtom(flags regexFlags) (runeSet, bool, rune) {
	if parser.position >= len(parser.input) {
		parser.fail()
		return nil, false, 0
	}
	if parser.input[parser.position] == '[' {
		remaining := string(parser.input[parser.position:])
		if strings.HasPrefix(remaining, "[:") {
			if end := strings.Index(remaining, ":]"); end >= 0 {
				name := remaining[2:end]
				isNegated := strings.HasPrefix(name, "^")
				name = strings.TrimPrefix(name, "^")
				if strings.Contains("|alnum|alpha|ascii|blank|cntrl|digit|graph|lower|print|punct|space|upper|word|xdigit|", "|"+name+"|") {
					parser.position += len([]rune(remaining[:end+2]))
					set := asciiClass(name)
					if flags.isInsensitive {
						set = foldSet(set, flags.isUnicode)
					}
					if isNegated {
						set = complementSet(set, flags.isUnicode)
					}
					return set, false, 0
				}
			}
		}
		return parser.class(flags), false, 0
	}
	if parser.take('\\') {
		literalFlags := flags
		literalFlags.isInsensitive = false
		node := parser.escape(literalFlags, true)
		isScalar := len(node.set) == 2 && node.set[0] == node.set[1]
		if isScalar {
			return node.set, true, node.set[0]
		}
		if flags.isInsensitive {
			node.set = foldSet(node.set, flags.isUnicode)
		}
		return node.set, false, 0
	}
	character := parser.input[parser.position]
	parser.position++
	return runeSet{character, character}, true, character
}
