// Package api defines transport compatibility values shared by HTTP and MCP.
package api

import (
	"strconv"
	"strings"
	"unicode"
)

// DebugString preserves serde's public invalid-type diagnostics for arbitrary strings.
func DebugString(value string) string {
	var output strings.Builder
	output.WriteByte('"')
	for _, character := range value {
		switch character {
		case 0:
			output.WriteString(`\0`)
		case '\n':
			output.WriteString(`\n`)
		case '\r':
			output.WriteString(`\r`)
		case '\t':
			output.WriteString(`\t`)
		case '\\', '"':
			output.WriteByte('\\')
			output.WriteRune(character)
		default:
			isExtended := unicode.Is(unicode.Mn, character) || unicode.Is(unicode.Me, character)
			if table := unicode.Properties["Other_Grapheme_Extend"]; table != nil && unicode.Is(table, character) {
				isExtended = true
			}
			if !unicode.IsPrint(character) || isExtended {
				output.WriteString(`\u{` + strconv.FormatInt(int64(character), 16) + `}`)
			} else {
				output.WriteRune(character)
			}
		}
	}
	output.WriteByte('"')
	return output.String()
}
