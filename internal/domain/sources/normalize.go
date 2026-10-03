package sources

import (
	"strings"
	"unicode"

	storage "github.com/QianFuv/LitRadar/internal/domain/storage"
	"golang.org/x/text/cases"
	"golang.org/x/text/language"
	"golang.org/x/text/unicode/norm"
)

// NormalizeText returns nonempty, trimmed NFC contract text.
func NormalizeText(value string) *string {
	value = strings.TrimSpace(norm.NFC.String(value))
	if value == "" {
		return nil
	}
	return &value
}

// PartialDate preserves real calendar precision after normalization.
type PartialDate struct {
	Value     string `json:"value"`
	Precision string `json:"precision"`
}

// NormalizeDate validates the original partial calendar grammar, including year zero.
func NormalizeDate(value string) *PartialDate {
	normalized := NormalizeText(value)
	if normalized == nil {
		return nil
	}
	precision := storage.DatePrecision(*normalized)
	if precision == nil {
		return nil
	}
	return &PartialDate{*normalized, *precision}
}

// Lowercase preserves Rust's Unicode string lowercase, including contextual final sigma.
func Lowercase(value string) string { return cases.Lower(language.Und).String(value) }

// NormalizeDoi validates a canonical bibliographic DOI after stripping one known prefix.
func NormalizeDoi(value string) *string {
	normalized := NormalizeText(value)
	if normalized == nil {
		return nil
	}
	value = Lowercase(*normalized)
	for _, prefix := range []string{"https://doi.org/", "http://doi.org/", "doi:"} {
		if strings.HasPrefix(value, prefix) {
			value = strings.TrimSpace(strings.TrimPrefix(value, prefix))
			break
		}
	}
	if !strings.HasPrefix(value, "10.") || !strings.Contains(value, "/") || strings.Contains(value, "://") || strings.ContainsAny(value, " \t\n\r\f") {
		return nil
	}
	return &value
}

// NormalizePmid validates ASCII digits and removes leading zeroes.
func NormalizePmid(value string) *string {
	normalized := NormalizeText(value)
	if normalized == nil {
		return nil
	}
	value = *normalized
	for _, character := range value {
		if character < '0' || character > '9' {
			return nil
		}
	}
	value = strings.TrimLeft(value, "0")
	if value == "" {
		value = "0"
	}
	return &value
}

// NormalizeIssn removes whitespace and hyphens, then validates the ISSN checksum.
func NormalizeIssn(value string) *string {
	normalized := NormalizeText(value)
	if normalized == nil {
		return nil
	}
	value = strings.Map(func(character rune) rune {
		if character == '-' || unicode.IsSpace(character) {
			return -1
		}
		return unicode.ToUpper(character)
	}, *normalized)
	if len(value) != 8 {
		return nil
	}
	sum := 0
	for index := 0; index < 7; index++ {
		if value[index] < '0' || value[index] > '9' {
			return nil
		}
		sum += int(value[index]-'0') * (8 - index)
	}
	check := (11 - sum%11) % 11
	expected := byte('0' + check)
	if check == 10 {
		expected = 'X'
	}
	if value[7] != expected {
		return nil
	}
	value = value[:4] + "-" + value[4:]
	return &value
}

// NormalizeBibliographicText collapses punctuation after scalar-wise lowercase and NFC.
func NormalizeBibliographicText(value string) string {
	var output strings.Builder
	for _, character := range norm.NFC.String(value) {
		if character == '\u0130' {
			output.WriteString("i ")
			continue
		}
		character = unicode.ToLower(character)
		if unicode.IsLetter(character) || unicode.IsNumber(character) || unicode.Is(unicode.Properties["Other_Alphabetic"], character) {
			output.WriteRune(character)
		} else {
			output.WriteByte(' ')
		}
	}
	return strings.Join(strings.Fields(output.String()), " ")
}

// NormalizeBibliographicLabel also removes leading decimal zeroes, retaining the legacy empty-to-zero rule.
func NormalizeBibliographicLabel(value string) string {
	value = NormalizeBibliographicText(value)
	for _, character := range value {
		if character < '0' || character > '9' {
			return value
		}
	}
	value = strings.TrimLeft(value, "0")
	if value == "" {
		return "0"
	}
	return value
}
