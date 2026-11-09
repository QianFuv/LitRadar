package logfilter

import "unicode/utf8"

type regexNode struct {
	kind      byte
	set       runeSet
	assertion string
	children  []*regexNode
	isLazy    bool
}

type instruction struct {
	kind            byte
	set             runeSet
	assertion       string
	next, alternate int
}

type fieldRegex struct {
	instructions []instruction
	start        int
}

func (expression *fieldRegex) emit(value instruction) int {
	position := len(expression.instructions)
	expression.instructions = append(expression.instructions, value)
	return position
}

// compile lowers regex nodes to ordered VM instructions.
func (expression *fieldRegex) compile(node *regexNode, next int) int {
	switch node.kind {
	case 'c', 'a':
		return expression.emit(instruction{kind: node.kind, set: node.set, assertion: node.assertion, next: next})
	case '|':
		return expression.compileAlternatives(node, next)
	case '*', '+':
		return expression.compileRepetition(node, next)
	case '?':
		return expression.compileOptional(node, next)
	default:
		for position := len(node.children) - 1; position >= 0; position-- {
			next = expression.compile(node.children[position], next)
		}
		return next
	}
}

// compileAlternatives emits earlier alternatives ahead of later alternatives.
func (expression *fieldRegex) compileAlternatives(node *regexNode, next int) int {
	result := expression.compile(node.children[len(node.children)-1], next)
	for position := len(node.children) - 2; position >= 0; position-- {
		result = expression.emit(instruction{kind: 's', next: expression.compile(node.children[position], next), alternate: result})
	}
	return result
}

// compileRepetition retains the loop back edge and greedy or lazy branch order.
func (expression *fieldRegex) compileRepetition(node *regexNode, next int) int {
	branch := expression.emit(instruction{kind: 's'})
	body := expression.compile(node.children[0], branch)
	choice := instruction{kind: 's', next: body, alternate: next}
	if node.isLazy {
		choice.next, choice.alternate = choice.alternate, choice.next
	}
	expression.instructions[branch] = choice
	if node.kind == '+' {
		return body
	}
	return branch
}

// compileOptional orders the optional body relative to its continuation.
func (expression *fieldRegex) compileOptional(node *regexNode, next int) int {
	body := expression.compile(node.children[0], next)
	choice := instruction{kind: 's', next: body, alternate: next}
	if node.isLazy {
		choice.next, choice.alternate = choice.alternate, choice.next
	}
	return expression.emit(choice)
}

// matches follows ordered leftmost-first paths and accepts only a match ending at EOI.
func (expression *fieldRegex) matches(value string) bool {
	if !utf8.ValidString(value) {
		return false
	}
	input := []rune(value)
	active := []int{}
	matchEnd := -1
	for position := 0; position <= len(input); position++ {
		pending := active
		if matchEnd < 0 {
			pending = append(pending, expression.start)
		}
		active = nil
		visited := make([]bool, len(expression.instructions))
		var expand func(int)
		isMatched := false
		expand = func(index int) {
			if visited[index] || isMatched {
				return
			}
			visited[index] = true
			current := expression.instructions[index]
			switch current.kind {
			case 's':
				expand(current.next)
				expand(current.alternate)
			case 'a':
				if regexAssertion(current.assertion, input, position) {
					expand(current.next)
				}
			case 'm':
				matchEnd = position
				isMatched = true
			case 'c':
				if position < len(input) && current.set.contains(input[position]) {
					active = append(active, current.next)
				}
			}
		}
		for _, index := range pending {
			expand(index)
			if isMatched {
				break
			}
		}
		if matchEnd >= 0 && len(active) == 0 {
			return matchEnd == len(input)
		}
	}
	return matchEnd == len(input)
}

// regexAssertion evaluates anchors against the adjacent input runes.
func regexAssertion(kind string, input []rune, position int) bool {
	previous, next := rune(-1), rune(-1)
	if position > 0 {
		previous = input[position-1]
	}
	if position < len(input) {
		next = input[position]
	}
	switch kind {
	case "start":
		return position == 0
	case "end":
		return position == len(input)
	case "line_start", "line_end":
		return lineAssertion(kind, previous, next, position == 0, position == len(input))
	case "crlf_start", "crlf_end":
		return crlfAssertion(kind, previous, next, position == 0, position == len(input))
	default:
		return wordAssertion(kind, previous, next)
	}
}

// lineAssertion recognizes LF line boundaries and input endpoints.
func lineAssertion(kind string, previous, next rune, isStart, isEnd bool) bool {
	if kind == "line_start" {
		return isStart || previous == '\n'
	}
	return isEnd || next == '\n'
}

// crlfAssertion recognizes line boundaries outside the middle of CRLF pairs.
func crlfAssertion(kind string, previous, next rune, isStart, isEnd bool) bool {
	if kind == "crlf_start" {
		return isStart || previous == '\n' || previous == '\r' && next != '\n'
	}
	return isEnd || next == '\r' || next == '\n' && previous != '\r'
}

// wordAssertion applies ASCII word membership even when the input contains Unicode.
func wordAssertion(kind string, previous, next rune) bool {
	isPreviousWord, isNextWord := asciiWord(previous), asciiWord(next)
	switch kind {
	case "boundary":
		return isPreviousWord != isNextWord
	case "not_boundary":
		return isPreviousWord == isNextWord
	case "word_start":
		return !isPreviousWord && isNextWord
	case "word_end":
		return isPreviousWord && !isNextWord
	}
	return false
}

// asciiWord recognizes underscore and the ASCII letter and digit ranges.
func asciiWord(value rune) bool {
	return value == '_' || value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9'
}
