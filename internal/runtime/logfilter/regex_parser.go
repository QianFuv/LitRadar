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

// expression assembles sequences and alternatives with persistent inline flags.
func (parser *regexParser) expression(flags regexFlags, isGroup bool) *regexNode {
	sequence := &regexNode{kind: 'q'}
	alternatives := []*regexNode{}
	for parser.err == nil {
		parser.skip(flags)
		if parser.expressionEnded(isGroup) {
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
			parser.quantify(sequence, character, flags)
			continue
		}
		node, nextFlags, isFlagOnly := parser.expressionAtom(character, flags)
		flags = nextFlags
		if !isFlagOnly {
			sequence.children = append(sequence.children, node)
		}
	}
	if len(alternatives) == 0 {
		return sequence
	}
	return &regexNode{kind: '|', children: append(alternatives, sequence)}
}

// expressionEnded consumes a closing group and checks whether termination is valid.
func (parser *regexParser) expressionEnded(isGroup bool) bool {
	if parser.position == len(parser.input) {
		if isGroup {
			parser.fail()
		}
		return true
	}
	if parser.take(')') {
		if !isGroup {
			parser.fail()
		}
		return true
	}
	return false
}

// quantify wraps the preceding atom using the current greedy polarity.
func (parser *regexParser) quantify(sequence *regexNode, character rune, flags regexFlags) {
	if len(sequence.children) == 0 {
		parser.fail()
		return
	}
	isLazy := parser.take('?') != flags.isUngreedy
	last := len(sequence.children) - 1
	sequence.children[last] = &regexNode{kind: byte(character), children: []*regexNode{sequence.children[last]}, isLazy: isLazy}
}

// expressionAtom parses an atom or a flag-only group that updates the surrounding flags.
func (parser *regexParser) expressionAtom(character rune, flags regexFlags) (*regexNode, regexFlags, bool) {
	switch character {
	case '(':
		return parser.group(flags)
	case '[':
		parser.position--
		return &regexNode{kind: 'c', set: parser.class(flags)}, flags, false
	case '\\':
		return parser.escape(flags, false), flags, false
	case '.':
		return dotRegex(flags), flags, false
	case '^', '$':
		return anchorRegex(character, flags), flags, false
	default:
		return literalRegex(character, flags), flags, false
	}
}

// group parses named and scoped groups or returns a persistent inline flag update.
func (parser *regexParser) group(flags regexFlags) (*regexNode, regexFlags, bool) {
	inner := flags
	parser.skip(flags)
	if parser.take('?') {
		if !parser.namedGroup() {
			inner = parser.groupFlags(inner)
			if parser.take(')') {
				return nil, inner, true
			}
			if !parser.take(':') {
				parser.fail()
			}
		}
	}
	return parser.expression(inner, true), flags, false
}

// namedGroup consumes either supported name prefix and its validated closing delimiter.
func (parser *regexParser) namedGroup() bool {
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
	}
	return isNamed
}

// groupFlags reads flag changes until the group delimiter without changing skip rules.
func (parser *regexParser) groupFlags(inner regexFlags) regexFlags {
	isEnabled := true
	for parser.position < len(parser.input) && parser.input[parser.position] != ':' && parser.input[parser.position] != ')' {
		flag := parser.input[parser.position]
		parser.position++
		parser.applyFlag(&inner, flag, &isEnabled)
	}
	return inner
}

// applyFlag updates one runtime flag, retaining disable mode after a minus sign.
func (parser *regexParser) applyFlag(flags *regexFlags, flag rune, isEnabled *bool) {
	switch flag {
	case '-':
		*isEnabled = false
	case 'i':
		flags.isInsensitive = *isEnabled
	case 'm':
		flags.isMultiline = *isEnabled
	case 's':
		flags.isDotAll = *isEnabled
	case 'U':
		flags.isUngreedy = *isEnabled
	case 'R':
		flags.isCrlf = *isEnabled
	case 'u':
		flags.isUnicode = *isEnabled
	case 'x':
		flags.isExtended = *isEnabled
	default:
		parser.fail()
	}
}

// dotRegex excludes line endings unless dot-all mode is active.
func dotRegex(flags regexFlags) *regexNode {
	set := runeSet{0, 0xd7ff, 0xe000, 0x10ffff}
	if !flags.isDotAll {
		excluded := runeSet{'\n', '\n'}
		if flags.isCrlf {
			excluded = append(excluded, '\r', '\r')
		}
		set = intersectSet(set, complementSet(normalizeSet(excluded), true))
	}
	return &regexNode{kind: 'c', set: set}
}

// anchorRegex selects input, LF or CRLF assertions from the active flags.
func anchorRegex(character rune, flags regexFlags) *regexNode {
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
	return &regexNode{kind: 'a', assertion: assertion}
}

func literalRegex(character rune, flags regexFlags) *regexNode {
	set := runeSet{character, character}
	if flags.isInsensitive {
		set = foldSet(set, flags.isUnicode)
	}
	return &regexNode{kind: 'c', set: set}
}

// escape decodes controls, assertions, hexadecimal literals and named classes.
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
		return parser.hexEscape(character, flags)
	case 'd', 'D', 's', 'S', 'w', 'W', 'p', 'P':
		return parser.classEscape(character, flags)
	default:
		return literalRegex(character, flags)
	}
}

// hexEscape decodes fixed-width hexadecimal escapes with extended skipping before digits.
func (parser *regexParser) hexEscape(character rune, flags regexFlags) *regexNode {
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
}

// classEscape selects frozen Unicode or ASCII classes before folding and negation.
func (parser *regexParser) classEscape(character rune, flags regexFlags) *regexNode {
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
}

// class parses set operands before applying folding, operations and negation.
func (parser *regexParser) class(flags regexFlags) runeSet {
	if !parser.take('[') {
		parser.fail()
		return nil
	}
	parser.skip(flags)
	isNegated := parser.take('^')
	parser.skip(flags)
	terms := parser.leadingClassTerms(flags)
	operands := []runeSet{}
	operators := []rune{}
	for parser.err == nil {
		parser.skip(flags)
		if parser.position == len(parser.input) {
			parser.fail()
			break
		}
		if parser.take(']') {
			break
		}
		if operator, exists := parser.classOperator(); exists {
			operands = append(operands, normalizeSet(terms))
			terms = nil
			operators = append(operators, operator)
			continue
		}
		set, isScalar, scalar := parser.classAtom(flags)
		parser.skip(flags)
		set = parser.classRange(set, isScalar, scalar, flags)
		terms = append(terms, set...)
	}
	operands = append(operands, normalizeSet(terms))
	return reconcileClass(operands, operators, flags, isNegated)
}

// leadingClassTerms recognizes literal hyphens and a leading literal closing bracket.
func (parser *regexParser) leadingClassTerms(flags regexFlags) runeSet {
	terms := runeSet{}
	for parser.take('-') {
		terms = append(terms, '-', '-')
		parser.skip(flags)
	}
	if len(terms) == 0 && parser.take(']') {
		terms = append(terms, ']', ']')
	}
	return terms
}

// classOperator consumes only doubled intersection, difference and symmetric difference tokens.
func (parser *regexParser) classOperator() (rune, bool) {
	character := parser.input[parser.position]
	if parser.position+1 < len(parser.input) && parser.input[parser.position+1] == character && strings.ContainsRune("&-~", character) {
		parser.position += 2
		return character, true
	}
	return 0, false
}

// classRange preserves scalar endpoint validation and rollback for a trailing hyphen.
func (parser *regexParser) classRange(set runeSet, isScalar bool, scalar rune, flags regexFlags) runeSet {
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
	return set
}

// reconcileClass folds each operand before left-associated operations, then folds and negates the result.
func reconcileClass(operands []runeSet, operators []rune, flags regexFlags, isNegated bool) runeSet {
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

// classAtom distinguishes nested classes, escaped endpoints and literal scalars.
func (parser *regexParser) classAtom(flags regexFlags) (runeSet, bool, rune) {
	if parser.position >= len(parser.input) {
		parser.fail()
		return nil, false, 0
	}
	if parser.input[parser.position] == '[' {
		if set, exists := parser.posixClass(flags); exists {
			return set, false, 0
		}
		return parser.class(flags), false, 0
	}
	if parser.take('\\') {
		return parser.escapedClassAtom(flags)
	}
	character := parser.input[parser.position]
	parser.position++
	return runeSet{character, character}, true, character
}

// posixClass recognizes supported POSIX names without consuming unknown nested-class forms.
func (parser *regexParser) posixClass(flags regexFlags) (runeSet, bool) {
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
				return set, true
			}
		}
	}
	return nil, false
}

// escapedClassAtom keeps literal endpoints scalar before folding nonscalar escape sets.
func (parser *regexParser) escapedClassAtom(flags regexFlags) (runeSet, bool, rune) {
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
