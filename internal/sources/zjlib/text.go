package zjlib

import (
	"slices"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

func decodeHtml(value string) string {
	var output strings.Builder
	for {
		start := strings.IndexByte(value, '&')
		if start < 0 {
			output.WriteString(value)
			break
		}
		output.WriteString(value[:start])
		value = value[start:]
		end := strings.IndexByte(value, ';')
		if end >= 0 {
			if decoded, ok := decodeEntity(value[1:end]); ok {
				output.WriteString(decoded)
				value = value[end+1:]
				continue
			}
		}
		output.WriteByte('&')
		value = value[1:]
	}
	return output.String()
}

// decodeEntity recognizes the original named entities before numeric scalar parsing.
func decodeEntity(entity string) (string, bool) {
	switch entity {
	case "amp":
		return "&", true
	case "lt":
		return "<", true
	case "gt":
		return ">", true
	case "quot":
		return "\"", true
	case "apos":
		return "'", true
	}
	return decodeNumericEntity(entity)
}
func stripTags(value string) string {
	var output strings.Builder
	isInsideTag := false
	for _, character := range value {
		switch character {
		case '<':
			isInsideTag = true
			output.WriteByte(' ')
		case '>':
			isInsideTag = false
		default:
			if !isInsideTag {
				output.WriteRune(character)
			}
		}
	}
	return decodeHtml(output.String())
}
func cleanText(value string) *string {
	text := strings.Join(strings.Fields(decodeHtml(value)), " ")
	if text == "" {
		return nil
	}
	return &text
}
func normalizeExact(value string) string {
	return strings.Map(func(character rune) rune {
		if unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) {
			return character
		}
		return -1
	}, domain.Lowercase(decodeHtml(value)))
}
func splitAuthors(value string) []string {
	decoded := decodeHtml(value)
	names := []string{}
	for _, part := range strings.FieldsFunc(decoded, func(character rune) bool { return strings.ContainsRune(";；,，、", character) }) {
		if name := normalizeExact(part); name != "" {
			names = append(names, name)
		}
	}
	if len(names) == 0 {
		if name := normalizeExact(decoded); name != "" {
			names = append(names, name)
		}
	}
	return names
}

// MatchesArticle requires nonempty normalized title, ordered authors and journal equality.
func MatchesArticle(expected, actual ArticleIdentity) bool {
	title, journal := normalizeExact(expected.Title), normalizeExact(expected.JournalTitle)
	authors := splitAuthors(expected.Authors)
	return title != "" && journal != "" && len(authors) > 0 && title == normalizeExact(actual.Title) && journal == normalizeExact(actual.JournalTitle) && slices.Equal(authors, splitAuthors(actual.Authors))
}

// SafeFilename retains at most 120 Unicode scalars after the legacy HTML and path cleanup.
func SafeFilename(value string) string {
	output := []rune{}
	wasSpace := false
	for _, character := range stripTags(value) {
		if strings.ContainsRune(`\/:*?"<>|`, character) {
			character = '_'
		}
		if unicode.IsSpace(character) {
			if !wasSpace {
				output = append(output, ' ')
			}
			wasSpace = true
		} else {
			output = append(output, character)
			wasSpace = false
		}
		if len(output) >= 120 {
			break
		}
	}
	result := strings.Trim(string(output), " .")
	if result == "" {
		return "cnki"
	}
	return result
}

// decodeNumericEntity accepts the legacy numeric prefixes, leading plus and Unicode scalars.
func decodeNumericEntity(entity string) (string, bool) {
	base := 10
	digits := ""
	if strings.HasPrefix(entity, "#x") || strings.HasPrefix(entity, "#X") {
		base = 16
		digits = entity[2:]
	} else if strings.HasPrefix(entity, "#") {
		digits = entity[1:]
	} else {
		return "", false
	}
	digits = strings.TrimPrefix(digits, "+")
	number, err := strconv.ParseUint(digits, base, 32)
	if err != nil || number > utf8.MaxRune || number >= 0xd800 && number <= 0xdfff {
		return "", false
	}
	return string(rune(number)), true
}
