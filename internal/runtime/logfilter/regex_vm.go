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

func (expression *fieldRegex) compile(node *regexNode, next int) int {
	switch node.kind {
	case 'c', 'a':
		return expression.emit(instruction{kind: node.kind, set: node.set, assertion: node.assertion, next: next})
	case '|':
		result := expression.compile(node.children[len(node.children)-1], next)
		for position := len(node.children) - 2; position >= 0; position-- {
			result = expression.emit(instruction{kind: 's', next: expression.compile(node.children[position], next), alternate: result})
		}
		return result
	case '*', '+':
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
	case '?':
		body := expression.compile(node.children[0], next)
		choice := instruction{kind: 's', next: body, alternate: next}
		if node.isLazy {
			choice.next, choice.alternate = choice.alternate, choice.next
		}
		return expression.emit(choice)
	default:
		for position := len(node.children) - 1; position >= 0; position-- {
			next = expression.compile(node.children[position], next)
		}
		return next
	}
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

func regexAssertion(kind string, input []rune, position int) bool {
	previous, next := rune(-1), rune(-1)
	if position > 0 {
		previous = input[position-1]
	}
	if position < len(input) {
		next = input[position]
	}
	isWord := func(value rune) bool {
		return value == '_' || value >= 'a' && value <= 'z' || value >= 'A' && value <= 'Z' || value >= '0' && value <= '9'
	}
	switch kind {
	case "start":
		return position == 0
	case "end":
		return position == len(input)
	case "line_start":
		return position == 0 || previous == '\n'
	case "line_end":
		return position == len(input) || next == '\n'
	case "crlf_start":
		return position == 0 || previous == '\n' || previous == '\r' && next != '\n'
	case "crlf_end":
		return position == len(input) || next == '\r' || next == '\n' && previous != '\r'
	case "boundary":
		return isWord(previous) != isWord(next)
	case "not_boundary":
		return isWord(previous) == isWord(next)
	case "word_start":
		return !isWord(previous) && isWord(next)
	case "word_end":
		return isWord(previous) && !isWord(next)
	}
	return false
}
