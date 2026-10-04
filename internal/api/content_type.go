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
	position := 0
	for position < len(parameters) {
		for position < len(parameters) && parameters[position] == ' ' {
			position++
		}
		if position == len(parameters) {
			return true
		}
		start := position
		for position < len(parameters) && mimeToken(parameters[position]) {
			position++
		}
		if position == start || position == len(parameters) || parameters[position] != '=' {
			return false
		}
		position++
		if position < len(parameters) && parameters[position] == '"' {
			position++
			start = position
			for position < len(parameters) {
				if parameters[position] == '"' && position > start {
					break
				}
				if parameters[position] < 32 || parameters[position] == 127 {
					return false
				}
				position++
			}
			if position == len(parameters) {
				return false
			}
			position++
			for position < len(parameters) && parameters[position] == ' ' {
				position++
			}
			if position == len(parameters) {
				return true
			}
			if parameters[position] != ';' {
				return false
			}
			position++
		} else {
			start = position
			for position < len(parameters) && mimeToken(parameters[position]) {
				position++
			}
			if position == len(parameters) {
				return true
			}
			if position == start || parameters[position] != ';' {
				return false
			}
			position++
		}
	}
	return true
}
func mimeToken(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("!#$%&'*+-.^_`|~", rune(character))
}
