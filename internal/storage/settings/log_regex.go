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
	sequence := logRegexSequence{}
	for parser.isValid {
		parser.skip(flags)
		if parser.expressionEnds(isGroup) {
			break
		}
		character := parser.input[parser.position]
		if character == '|' {
			sequence.alternatives = append(sequence.alternatives, concatenationDepth(sequence.terms))
			sequence.terms = nil
			sequence.isLastFlags = false
			parser.position++
			continue
		}
		if strings.ContainsRune("*+?", character) {
			sequence.quantify(parser)
			continue
		}
		depth, nextFlags, isLastFlags := parser.expressionAtom(flags, nesting)
		flags = nextFlags
		sequence.isLastFlags = isLastFlags
		sequence.terms = append(sequence.terms, depth)
	}
	depth := sequence.depth()
	if depth > 250 {
		parser.isValid = false
	}
	return depth
}

// logRegexSequence retains concatenation, alternatives and flag-only quantifier state.
type logRegexSequence struct {
	terms        []int
	alternatives []int
	isLastFlags  bool
}

// concatenationDepth accounts for the explicit concatenation node only for multiple terms.
func concatenationDepth(terms []int) int {
	depth := 0
	for _, item := range terms {
		depth = max(depth, item)
	}
	if len(terms) > 1 {
		depth++
	}
	return depth
}

// depth combines alternatives after preserving each concatenation's own depth.
func (sequence logRegexSequence) depth() int {
	depth := concatenationDepth(sequence.terms)
	if len(sequence.alternatives) > 0 {
		for _, alternative := range sequence.alternatives {
			depth = max(depth, alternative)
		}
		depth++
	}
	return depth
}

// quantify preserves repeated quantifiers and rejects a missing or flag-only predecessor.
func (sequence *logRegexSequence) quantify(parser *logRegexParser) {
	if len(sequence.terms) == 0 || sequence.isLastFlags {
		parser.isValid = false
		return
	}
	parser.position++
	sequence.terms[len(sequence.terms)-1]++
	parser.take('?')
}

// expressionEnds consumes a group closer while rejecting a missing or unexpected closer.
func (parser *logRegexParser) expressionEnds(isGroup bool) bool {
	if parser.position == len(parser.input) {
		if isGroup {
			parser.isValid = false
		}
		return true
	}
	if parser.peek(')') {
		if !isGroup {
			parser.isValid = false
		} else {
			parser.position++
		}
		return true
	}
	return false
}

// expressionAtom handles one production and keeps unscoped flag changes visible to later terms.
func (parser *logRegexParser) expressionAtom(flags logRegexFlags, nesting int) (int, logRegexFlags, bool) {
	depth := 0
	switch parser.input[parser.position] {
	case '{', '}':
		parser.isValid = false
	case '(':
		return parser.group(flags, nesting)
	case '[':
		depth = parser.class(flags, nesting+1).depth
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
	return depth, flags, false
}

// group separates named captures, scoped flags and persistent flag-only groups.
func (parser *logRegexParser) group(flags logRegexFlags, nesting int) (int, logRegexFlags, bool) {
	parser.position++
	parser.skip(flags)
	if !parser.take('?') {
		return parser.expression(flags, true, nesting+1) + 1, flags, false
	}
	if parser.take('P') {
		if !parser.take('<') {
			parser.isValid = false
			return 0, flags, false
		}
		parser.captureName()
		return parser.expression(flags, true, nesting+1) + 1, flags, false
	}
	if parser.take('<') {
		parser.captureName()
		return parser.expression(flags, true, nesting+1) + 1, flags, false
	}
	inner, count := parser.groupFlags(flags)
	if parser.take(')') {
		if count == 0 {
			parser.isValid = false
		}
		return 0, inner, true
	}
	if parser.take(':') {
		return parser.expression(inner, true, nesting+1) + 1, flags, false
	}
	parser.isValid = false
	return 0, flags, false
}

// groupFlags rejects repeated/unknown flags and a final negation marker without a flag.
func (parser *logRegexParser) groupFlags(flags logRegexFlags) (logRegexFlags, int) {
	seen := map[rune]bool{}
	isNegative, isLastNegative := false, false
	for parser.position < len(parser.input) && !parser.peek(':') && !parser.peek(')') {
		flag := parser.input[parser.position]
		parser.position++
		if seen[flag] || !strings.ContainsRune("imsURux-", flag) {
			parser.isValid = false
			break
		}
		seen[flag] = true
		if flag == '-' {
			isNegative, isLastNegative = true, true
			continue
		}
		isLastNegative = false
		flags.set(flag, !isNegative)
	}
	if isLastNegative {
		parser.isValid = false
	}
	return flags, len(seen)
}

// set applies only flags relevant to grammar validity and byte-set validation.
func (flags *logRegexFlags) set(flag rune, isEnabled bool) {
	switch flag {
	case 'u':
		flags.isUnicode = isEnabled
	case 'x':
		flags.isExtended = isEnabled
	case 'i':
		flags.isInsensitive = isEnabled
	}
}

func (parser *logRegexParser) captureName() {
	start := parser.position
	for parser.position < len(parser.input) && !parser.peek('>') {
		if !isCaptureNameRune(parser.input[parser.position], parser.position == start) {
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

// isCaptureNameRune retains distinct first-character and subsequent punctuation rules.
func isCaptureNameRune(character rune, isFirst bool) bool {
	if isFirst {
		return character == '_' || isAlphabetic(character)
	}
	return character == '_' || character == '.' || character == '[' || character == ']' || isAlphabetic(character) || unicode.IsNumber(character)
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
	if atom, isControl := controlEscapeAtom(character); isControl {
		return atom
	}
	return parser.typedEscape(character, flags, isClass)
}

// controlEscapeAtom expands the six supported single-letter control escapes.
func controlEscapeAtom(character rune) (logRegexAtom, bool) {
	switch character {
	case 'a':
		return literalAtom(7), true
	case 'f':
		return literalAtom(12), true
	case 't':
		return literalAtom(9), true
	case 'n':
		return literalAtom(10), true
	case 'r':
		return literalAtom(13), true
	case 'v':
		return literalAtom(11), true
	default:
		return logRegexAtom{}, false
	}
}

// typedEscape separates numeric, byte-set, property and assertion escape productions.
func (parser *logRegexParser) typedEscape(character rune, flags logRegexFlags, isClass bool) logRegexAtom {
	switch {
	case strings.ContainsRune("xuU", character):
		return parser.hexEscape(character, flags)
	case strings.ContainsRune("dDsSwW", character):
		if !flags.isUnicode && strings.ContainsRune("DSW", character) {
			parser.isValid = false
		}
		return shorthandAtom(character)
	case character == 'p' || character == 'P':
		parser.propertyEscape(flags)
	default:
		return parser.assertionEscape(character, flags, isClass)
	}
	return logRegexAtom{}
}

// assertionEscape distinguishes zero-width assertions from escaped literal punctuation.
func (parser *logRegexParser) assertionEscape(character rune, flags logRegexFlags, isClass bool) logRegexAtom {
	switch {
	case strings.ContainsRune("bB<>", character):
		if isClass || flags.isUnicode {
			parser.isValid = false
		}
	case character == 'A' || character == 'z':
		if isClass {
			parser.isValid = false
		}
	default:
		return parser.literalEscape(character)
	}
	return logRegexAtom{}
}

// hexEscape reads fixed-width hexadecimal scalars with extended-mode whitespace skipping.
func (parser *logRegexParser) hexEscape(character rune, flags logRegexFlags) logRegexAtom {
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
	if err != nil || !isEscapedScalar(value, character, flags) {
		parser.isValid = false
	}
	return literalAtom(rune(value))
}

// isEscapedScalar excludes surrogates and byte-mode hexadecimal values above ASCII.
func isEscapedScalar(value uint64, character rune, flags logRegexFlags) bool {
	return value <= 0x10ffff && !(value >= 0xd800 && value <= 0xdfff) && (flags.isUnicode || character != 'x' || value <= 127)
}

// shorthandAtom records ASCII byte membership and its complemented variants.
func shorthandAtom(character rune) logRegexAtom {
	result := logRegexAtom{}
	for index := range result.bytes {
		result.bytes[index] = isShorthandByte(character, rune(index))
		if strings.ContainsRune("DSW", character) {
			result.bytes[index] = !result.bytes[index]
		}
	}
	return result
}

// isShorthandByte retains the validator's ASCII digit, space and word sets.
func isShorthandByte(character, value rune) bool {
	switch character {
	case 'd', 'D':
		return value >= '0' && value <= '9'
	case 's', 'S':
		return strings.ContainsRune(" \t\n\r\v\f", value)
	case 'w', 'W':
		return value == '_' || isAsciiAlphanumeric(value)
	default:
		return false
	}
}

// propertyEscape accepts only a single general-category letter in Unicode mode.
func (parser *logRegexParser) propertyEscape(flags logRegexFlags) {
	parser.skip(flags)
	if !flags.isUnicode || parser.position >= len(parser.input) || !strings.ContainsRune("LMNPSZClmnpszc", parser.input[parser.position]) {
		parser.isValid = false
	} else {
		parser.position++
	}
}

// literalEscape rejects unknown alphabetic, numeric and non-ASCII escapes.
func (parser *logRegexParser) literalEscape(character rune) logRegexAtom {
	if character > 127 || unicode.IsLetter(character) || unicode.IsNumber(character) {
		parser.isValid = false
	}
	return literalAtom(character)
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
	terms := parser.leadingClassTerms(flags)
	operands := []logRegexAtom{}
	operators := []rune{}
	for parser.isValid {
		parser.skip(flags)
		if parser.position >= len(parser.input) {
			parser.isValid = false
			break
		}
		if parser.take(']') {
			break
		}
		if operator, hasOperator := parser.takeClassOperator(); hasOperator {
			operands = append(operands, unionClassTerms(terms))
			operators = append(operators, operator)
			terms = nil
			continue
		}
		terms = append(terms, parser.classTerm(flags, nesting))
	}
	operands = append(operands, unionClassTerms(terms))
	result = combineClassOperands(operands, operators)
	return parser.finishClass(result, flags, isNegated)
}

// leadingClassTerms retains literal leading hyphens and a first closing bracket.
func (parser *logRegexParser) leadingClassTerms(flags logRegexFlags) []logRegexAtom {
	terms := []logRegexAtom{}
	for parser.take('-') {
		terms = append(terms, literalAtom('-'))
		parser.skip(flags)
	}
	if len(terms) == 0 && parser.take(']') {
		terms = append(terms, literalAtom(']'))
	}
	return terms
}

// unionClassTerms merges adjacent atoms and accounts for a multi-term union node.
func unionClassTerms(terms []logRegexAtom) logRegexAtom {
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

// takeClassOperator recognizes paired set operators before range/atom parsing.
func (parser *logRegexParser) takeClassOperator() (rune, bool) {
	character := parser.input[parser.position]
	if parser.position+1 < len(parser.input) && parser.input[parser.position+1] == character && strings.ContainsRune("&-~", character) {
		parser.position += 2
		return character, true
	}
	return 0, false
}

// classTerm preserves range-end lookahead and rejects non-ASCII scalar atoms in byte mode.
func (parser *logRegexParser) classTerm(flags logRegexFlags, nesting int) logRegexAtom {
	atom := parser.classAtom(flags, nesting)
	parser.skip(flags)
	if parser.hasRangeEnd(flags) {
		parser.position++
		parser.skip(flags)
		right := parser.classAtom(flags, nesting)
		atom = parser.rangeAtom(atom, right, flags)
	}
	if !flags.isUnicode && atom.isScalar && atom.scalar > 127 {
		parser.isValid = false
	}
	return atom
}

// rangeAtom accepts only ordered scalar endpoints and preserves byte-mode ASCII limits.
func (parser *logRegexParser) rangeAtom(left, right logRegexAtom, flags logRegexFlags) logRegexAtom {
	if !left.isScalar || !right.isScalar || left.scalar > right.scalar || !flags.isUnicode && (left.scalar > 127 || right.scalar > 127) {
		parser.isValid = false
		return left
	}
	for value := max(left.scalar, 0); value <= min(right.scalar, 255); value++ {
		left.bytes[value] = true
	}
	left.isScalar = false
	return left
}

// combineClassOperands applies operators left to right and records each operation depth.
func combineClassOperands(operands []logRegexAtom, operators []rune) logRegexAtom {
	result := operands[0]
	for index, operator := range operators {
		right := operands[index+1]
		result.depth = max(result.depth, right.depth) + 1
		for value := range result.bytes {
			result.bytes[value] = combineClassByte(result.bytes[value], right.bytes[value], operator)
		}
	}
	return result
}

// combineClassByte evaluates the recognized intersection, subtraction and symmetric difference.
func combineClassByte(left, right bool, operator rune) bool {
	switch operator {
	case '&':
		return left && right
	case '-':
		return left && !right
	case '~':
		return left != right
	default:
		return left
	}
}

// finishClass preserves folding, negation, byte-mode rejection and enclosing depth order.
func (parser *logRegexParser) finishClass(result logRegexAtom, flags logRegexFlags, isNegated bool) logRegexAtom {
	if flags.isInsensitive {
		foldClassBytes(&result)
	}
	if isNegated {
		for value := range result.bytes {
			result.bytes[value] = !result.bytes[value]
		}
	}
	if !flags.isUnicode && hasNonAsciiBytes(result) {
		parser.isValid = false
	}
	result.depth++
	result.isScalar = false
	if result.depth > 250 {
		parser.isValid = false
	}
	return result
}

// foldClassBytes joins ASCII letter case pairs after all set operators.
func foldClassBytes(result *logRegexAtom) {
	for value := 'a'; value <= 'z'; value++ {
		if result.bytes[value] || result.bytes[value-32] {
			result.bytes[value] = true
			result.bytes[value-32] = true
		}
	}
}

// hasNonAsciiBytes detects any byte membership forbidden outside Unicode mode.
func hasNonAsciiBytes(result logRegexAtom) bool {
	for value := 128; value < 256; value++ {
		if result.bytes[value] {
			return true
		}
	}
	return false
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
		if atom, isPosix := parser.posixClass(flags); isPosix {
			return atom
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

// posixClass consumes only recognized complete POSIX syntax, leaving other forms to nested classes.
func (parser *logRegexParser) posixClass(flags logRegexFlags) (logRegexAtom, bool) {
	remaining := string(parser.input[parser.position:])
	if !strings.HasPrefix(remaining, "[:") {
		return logRegexAtom{}, false
	}
	end := strings.Index(remaining, ":]")
	if end < 0 {
		return logRegexAtom{}, false
	}
	name := remaining[2:end]
	isNegated := strings.HasPrefix(name, "^")
	name = strings.TrimPrefix(name, "^")
	if !strings.Contains("|alnum|alpha|ascii|blank|cntrl|digit|graph|lower|print|punct|space|upper|word|xdigit|", "|"+name+"|") {
		return logRegexAtom{}, false
	}
	if isNegated && !flags.isUnicode {
		parser.isValid = false
	}
	parser.position += utf8.RuneCountInString(remaining[:end+2])
	result := logRegexAtom{}
	for index := range result.bytes {
		result.bytes[index] = posixByteMatches(name, index) != isNegated
	}
	return result, true
}

// posixByteMatches defines the byte membership of each supported POSIX class.
func posixByteMatches(name string, index int) bool {
	switch name {
	case "alnum":
		return isAsciiAlphanumeric(rune(index))
	case "alpha":
		return isAsciiLetter(rune(index))
	case "digit":
		return isAsciiDigit(rune(index))
	case "lower":
		return isAsciiLowercase(rune(index))
	case "upper":
		return isAsciiUppercase(rune(index))
	case "word":
		return isAsciiAlphanumeric(rune(index)) || index == '_'
	case "xdigit":
		return isAsciiHexDigit(rune(index))
	case "ascii":
		return index < 128
	case "blank":
		return index == ' ' || index == '\t'
	case "cntrl":
		return index < 32 || index == 127
	case "space":
		return strings.ContainsRune(" \t\n\r\v\f", rune(index))
	case "graph":
		return index >= 33 && index <= 126
	case "print":
		return index >= 32 && index <= 126
	case "punct":
		return index >= 33 && index <= 126 && !isAsciiAlphanumeric(rune(index))
	default:
		return false
	}
}

// isAsciiDigit tests the decimal ASCII range used by POSIX and byte-only regex sets.
func isAsciiDigit(character rune) bool {
	return character >= '0' && character <= '9'
}

// isAsciiLetter tests both case ranges without adding Unicode alphabetic membership.
func isAsciiLetter(character rune) bool {
	return isAsciiLowercase(character) || isAsciiUppercase(character)
}

// isAsciiLowercase tests only the lowercase byte range used by POSIX classes.
func isAsciiLowercase(character rune) bool {
	return character >= 'a' && character <= 'z'
}

// isAsciiUppercase tests only the uppercase byte range used by POSIX classes.
func isAsciiUppercase(character rune) bool {
	return character >= 'A' && character <= 'Z'
}

// isAsciiAlphanumeric joins the literal ASCII digit and letter sets.
func isAsciiAlphanumeric(character rune) bool {
	return isAsciiDigit(character) || isAsciiLetter(character)
}

// isAsciiHexDigit retains only the decimal digits and first six ASCII letters of each case.
func isAsciiHexDigit(character rune) bool {
	return isAsciiDigit(character) || character >= 'a' && character <= 'f' || character >= 'A' && character <= 'F'
}
