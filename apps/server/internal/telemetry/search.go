package telemetry

import (
	"strings"
	"unicode"
)

// Magnitude reduces result counts to the fixed, anonymous search buckets.
func Magnitude(count int64) string {
	switch {
	case count <= 0:
		return "0"
	case count < 10:
		return "1-9"
	case count < 100:
		return "10-99"
	case count < 1000:
		return "100-999"
	default:
		return "1000+"
	}
}

// SearchShape discards query text after extracting coarse lexical features.
// Simple mode treats operators and stars literally. Advanced mode recognizes
// operators and prefix stars only outside quoted phrases.
func SearchShape(endpoint, mode, database, text string) Dimensions {
	shape := Dimensions{Kind: "search", Name: endpoint, Mode: mode, Database: database, HitCountBasis: "returned"}
	count := 0
	if mode != "advanced" {
		shape.Mode = "simple"
		count = len(strings.Fields(text))
	} else {
		isQuoted, isColumnGroup, isDistance := false, false, false
		var token strings.Builder
		consume := func() {
			word := token.String()
			if word == "AND" || word == "OR" || word == "NOT" || word == "NEAR" {
				shape.UsesBoolean = true
			} else if word != "" && !isDistance {
				count++
			}
			token.Reset()
		}
		for _, character := range text {
			if isQuoted {
				if character == '"' {
					if token.Len() > 0 {
						count++
					}
					token.Reset()
					isQuoted = false
				} else {
					token.WriteRune(character)
				}
				continue
			}
			if character == '{' {
				consume()
				isColumnGroup = true
				continue
			}
			if character == '}' {
				isColumnGroup = false
				continue
			}
			if isColumnGroup {
				continue
			}
			if character == ':' {
				token.Reset()
				continue
			}
			if character == '"' {
				consume()
				isQuoted = true
				continue
			}
			if character == '*' {
				shape.UsesPrefix = true
				consume()
				continue
			}
			if character == ',' {
				consume()
				isDistance = true
				continue
			}
			if character == ')' {
				consume()
				isDistance = false
				continue
			}
			if unicode.IsSpace(character) || strings.ContainsRune("(+^", character) {
				consume()
				continue
			}
			token.WriteRune(character)
		}
		if isQuoted {
			if token.Len() > 0 {
				count++
			}
		} else {
			consume()
		}
	}
	switch {
	case count == 0:
		shape.TermCount = "0"
	case count == 1:
		shape.TermCount = "1"
	case count <= 4:
		shape.TermCount = "2-4"
	case count <= 9:
		shape.TermCount = "5-9"
	default:
		shape.TermCount = "10+"
	}
	return shape
}
