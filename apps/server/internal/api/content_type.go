package api

import "strings"

// isJsonContentType retains mime 0.3's parameter grammar without collapsing duplicates.
func isJsonContentType(value string) bool {
	if !validHeaderText(value) {
		return false
	}
	media, parameters, hasParameters := strings.Cut(value, ";")
	primary, subtype, hasSlash := strings.Cut(media, "/")
	if !hasSlash || asciiLower(primary) != "application" {
		return false
	}
	for _, character := range []byte(subtype) {
		if !mimeToken(character) {
			return false
		}
	}
	subtype = asciiLower(subtype)
	if subtype != "json" && !(strings.LastIndexByte(subtype, '+') > 0 && strings.HasSuffix(subtype, "+json")) {
		return false
	}
	if !hasParameters {
		return true
	}
	return validMimeParameters(parameters)
}

type mimeParameterParser struct {
	input    string
	position int
}

// validMimeParameters retains duplicate parameters and literal-space skipping.
func validMimeParameters(parameters string) bool {
	parser := mimeParameterParser{input: parameters}
	for parser.position < len(parser.input) {
		parser.spaces()
		if parser.position == len(parser.input) {
			return true
		}
		if !parser.name() {
			return false
		}
		if !parser.value() {
			return false
		}
	}
	return true
}

// spaces consumes only ASCII spaces accepted by the MIME parameter grammar.
func (parser *mimeParameterParser) spaces() {
	for parser.position < len(parser.input) && parser.input[parser.position] == ' ' {
		parser.position++
	}
}

// token consumes contiguous MIME token bytes and returns their starting offset.
func (parser *mimeParameterParser) token() int {
	start := parser.position
	for parser.position < len(parser.input) && mimeToken(parser.input[parser.position]) {
		parser.position++
	}
	return start
}

// name requires a nonempty token immediately followed by an equals sign.
func (parser *mimeParameterParser) name() bool {
	start := parser.token()
	if parser.position == start || parser.position == len(parser.input) || parser.input[parser.position] != '=' {
		return false
	}
	parser.position++
	return true
}

// value distinguishes quoted parameters from unquoted tokens, retaining empty-at-EOF acceptance.
func (parser *mimeParameterParser) value() bool {
	if parser.position < len(parser.input) && parser.input[parser.position] == '"' {
		return parser.quoted()
	}
	start := parser.token()
	if parser.position == len(parser.input) {
		return true
	}
	if parser.position == start || parser.input[parser.position] != ';' {
		return false
	}
	parser.position++
	return true
}

// quoted retains the nonempty closing-quote rule and byte-level control rejection.
func (parser *mimeParameterParser) quoted() bool {
	parser.position++
	start := parser.position
	for parser.position < len(parser.input) {
		if parser.input[parser.position] == '"' && parser.position > start {
			break
		}
		if parser.input[parser.position] < 32 || parser.input[parser.position] == 127 {
			return false
		}
		parser.position++
	}
	if parser.position == len(parser.input) {
		return false
	}
	parser.position++
	return parser.quotedEnd()
}

// quotedEnd permits spaces before EOF or a following parameter delimiter.
func (parser *mimeParameterParser) quotedEnd() bool {
	parser.spaces()
	if parser.position == len(parser.input) {
		return true
	}
	if parser.input[parser.position] != ';' {
		return false
	}
	parser.position++
	return true
}

func mimeToken(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("!#$%&'*+-.^_`|~", rune(character))
}
