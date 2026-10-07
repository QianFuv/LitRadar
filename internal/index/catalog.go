// Package index coordinates provider-neutral indexing and durable worker acknowledgements.
package index

import (
	"errors"
	"fmt"
	"os"
	"slices"
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
	"github.com/QianFuv/LitRadar/internal/provider"
)

var catalogColumns = []string{"catalog_id", "catalog_aliases", "title", "issn", "eissn", "all_issns", "title_aliases", "area", "utd_rank", "utd_rating", "abs_rank", "abs_rating", "fms_rank", "fms_rating", "fmscn_rank", "fmscn_rating"}

// ReadCatalogCsv reads a UTF-8 maintained catalog and validates every identity before indexing.
func ReadCatalogCsv(path string) ([]domain.JournalCatalogEntry, error) {
	body, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	if !utf8.Valid(body) {
		return nil, errors.New("stream did not contain valid UTF-8")
	}
	return ParseCatalogCsv(string(body))
}

// ParseCatalogCsv preserves the historical single-line quote grammar and filtered row numbering.
func ParseCatalogCsv(text string) ([]domain.JournalCatalogEntry, error) {
	lines := []string{}
	for _, line := range strings.Split(text, "\n") {
		if strings.TrimSpace(line) != "" {
			lines = append(lines, line)
		}
	}
	if len(lines) == 0 {
		return nil, errors.New("canonical catalog is empty")
	}
	headers, err := parseCatalogLine(lines[0])
	if err != nil {
		return nil, err
	}
	if !slices.Equal(headers, catalogColumns) {
		return nil, fmt.Errorf("catalog must use exact v3 header %s; found %s", strings.Join(catalogColumns, ","), strings.Join(headers, ","))
	}
	rows := make([]map[string]string, 0, len(lines)-1)
	for position, line := range lines[1:] {
		values, err := parseCatalogLine(line)
		if err != nil {
			return nil, fmt.Errorf("catalog row %d: %w", position+2, err)
		}
		if len(values) != len(headers) {
			return nil, fmt.Errorf("catalog row %d has %d columns; expected %d", position+2, len(values), len(headers))
		}
		row := map[string]string{}
		for column, name := range headers {
			row[name] = values[column]
		}
		rows = append(rows, row)
	}
	return BuildCatalogEntries(rows)
}

func parseCatalogLine(line string) ([]string, error) {
	characters := []rune(strings.TrimRight(line, "\r"))
	values := []string{}
	var current strings.Builder
	isQuoted := false
	for position := 0; position < len(characters); position++ {
		character := characters[position]
		switch {
		case character == '"' && isQuoted && position+1 < len(characters) && characters[position+1] == '"':
			current.WriteRune('"')
			position++
		case character == '"':
			isQuoted = !isQuoted
		case character == ',' && !isQuoted:
			values = append(values, strings.TrimSpace(current.String()))
			current.Reset()
		default:
			current.WriteRune(character)
		}
	}
	if isQuoted {
		return nil, errors.New("catalog CSV row has an unterminated quoted field")
	}
	return append(values, strings.TrimSpace(current.String())), nil
}

// BuildCatalogEntry normalizes a complete v3 row in its original validation order.
func BuildCatalogEntry(row map[string]string) (domain.JournalCatalogEntry, error) {
	entry := domain.JournalCatalogEntry{}
	if err := validateCatalogColumns(row); err != nil {
		return entry, err
	}
	catalogId := domain.NormalizeText(row["catalog_id"])
	if catalogId == nil {
		return entry, errors.New("catalog_id must not be blank")
	}
	if *catalogId != row["catalog_id"] {
		return entry, errors.New("catalog_id must already use canonical trimmed form")
	}
	entry.CatalogId = *catalogId
	aliases, err := catalogList(row, "catalog_aliases", true, false)
	if err != nil {
		return entry, err
	}
	entry.CatalogAliases = aliases
	title := domain.NormalizeText(row["title"])
	if title == nil {
		return entry, errors.New("title must not be blank")
	}
	entry.Title = *title
	if err := normalizeCatalogPrimaryIssns(row, &entry); err != nil {
		return entry, err
	}
	entry.AllIssns, err = catalogList(row, "all_issns", false, true)
	if err != nil {
		return entry, err
	}
	entry.TitleAliases, err = catalogList(row, "title_aliases", false, false)
	if err != nil {
		return entry, err
	}
	entry.Area = domain.NormalizeText(row["area"])
	entry.Rankings = domain.JournalRankings{UtdRank: domain.NormalizeText(row["utd_rank"]), UtdRating: domain.NormalizeText(row["utd_rating"]), AbsRank: domain.NormalizeText(row["abs_rank"]), AbsRating: domain.NormalizeText(row["abs_rating"]), FmsRank: domain.NormalizeText(row["fms_rank"]), FmsRating: domain.NormalizeText(row["fms_rating"]), FmscnRank: domain.NormalizeText(row["fmscn_rank"]), FmscnRating: domain.NormalizeText(row["fmscn_rating"])}
	if err := provider.ValidateCatalogEntry(entry); err != nil {
		return domain.JournalCatalogEntry{}, err
	}
	return entry, nil
}

func catalogList(row map[string]string, field string, isIdentity, isIssn bool) ([]string, error) {
	values := []string{}
	for _, raw := range strings.Split(row[field], ";") {
		value, err := catalogListValue(raw, field, isIdentity, isIssn)
		if err != nil {
			return nil, err
		}
		if value == nil {
			continue
		}
		if slices.Contains(values, *value) {
			if isIssn {
				continue
			}
			return nil, fmt.Errorf("%s contains a duplicate value", field)
		}
		values = append(values, *value)
	}
	return values, nil
}

// BuildCatalogEntries rejects overlapping catalog aliases and ISSN ownership across all rows.
func BuildCatalogEntries(rows []map[string]string) ([]domain.JournalCatalogEntry, error) {
	if len(rows) == 0 {
		return nil, errors.New("canonical catalog must contain at least one journal")
	}
	type owner struct {
		row     int
		catalog string
	}
	catalogOwners, issnOwners := map[string]owner{}, map[string]owner{}
	entries := make([]domain.JournalCatalogEntry, 0, len(rows))
	for position, row := range rows {
		rowNumber := position + 2
		entry, err := BuildCatalogEntry(row)
		if err != nil {
			return nil, fmt.Errorf("catalog row %d: %w", rowNumber, err)
		}
		for _, value := range append([]string{entry.CatalogId}, entry.CatalogAliases...) {
			if previous, ok := catalogOwners[value]; ok {
				return nil, fmt.Errorf("catalog row %d catalog identity %s is already owned by row %d (%s)", rowNumber, value, previous.row, previous.catalog)
			}
			catalogOwners[value] = owner{rowNumber, entry.CatalogId}
		}
		for _, value := range entry.AllIssns {
			if previous, ok := issnOwners[value]; ok {
				return nil, fmt.Errorf("catalog row %d ISSN %s is already owned by row %d (%s)", rowNumber, value, previous.row, previous.catalog)
			}
			issnOwners[value] = owner{rowNumber, entry.CatalogId}
		}
		entries = append(entries, entry)
	}
	return entries, nil
}

func debugStrings(values []string) string {
	quoted := make([]string, len(values))
	for position, value := range values {
		var text strings.Builder
		text.WriteByte('"')
		for _, character := range value {
			writeDebugCharacter(&text, character)
		}
		text.WriteByte('"')
		quoted[position] = text.String()
	}
	return "[" + strings.Join(quoted, ", ") + "]"
}

// validateCatalogColumns reports sorted missing and unexpected names before decoding any row field.
func validateCatalogColumns(row map[string]string) error {
	missing, unexpected := []string{}, []string{}
	for _, column := range catalogColumns {
		if _, ok := row[column]; !ok {
			missing = append(missing, column)
		}
	}
	for column := range row {
		if !slices.Contains(catalogColumns, column) {
			unexpected = append(unexpected, column)
		}
	}
	if len(missing) > 0 || len(unexpected) > 0 {
		slices.Sort(missing)
		slices.Sort(unexpected)
		return fmt.Errorf("catalog row must use exact v3 columns; missing=%s, unexpected=%s", debugStrings(missing), debugStrings(unexpected))
	}
	return nil
}

// normalizeCatalogPrimaryIssns preserves ISSN-before-eISSN admission and partial row state on failure.
func normalizeCatalogPrimaryIssns(row map[string]string, entry *domain.JournalCatalogEntry) error {
	for _, field := range []struct {
		name        string
		destination **string
	}{{"issn", &entry.Issn}, {"eissn", &entry.Eissn}} {
		if text := domain.NormalizeText(row[field.name]); text != nil {
			value := domain.NormalizeIssn(*text)
			if value == nil {
				return fmt.Errorf("%s contains an invalid ISSN", field.name)
			}
			*field.destination = value
		}
	}
	return nil
}

// catalogListValue preserves blank omission, canonical identity admission and optional ISSN normalization.
func catalogListValue(raw, field string, isIdentity, isIssn bool) (*string, error) {
	value := domain.NormalizeText(raw)
	if value == nil {
		return nil, nil
	}
	if isIdentity && *value != raw {
		return nil, fmt.Errorf("%s must already use canonical trimmed form", field)
	}
	if isIssn {
		value = domain.NormalizeIssn(*value)
		if value == nil {
			return nil, fmt.Errorf("%s contains an invalid ISSN", field)
		}
	}
	return value, nil
}

// writeDebugCharacter preserves Rust-style diagnostic escaping for each observed Unicode rune.
func writeDebugCharacter(text *strings.Builder, character rune) {
	switch character {
	case 0:
		text.WriteString(`\0`)
	case '\n':
		text.WriteString(`\n`)
	case '\r':
		text.WriteString(`\r`)
	case '\t':
		text.WriteString(`\t`)
	case '\\', '"':
		text.WriteByte('\\')
		text.WriteRune(character)
	default:
		if !unicode.IsPrint(character) || isExtendedDebugCharacter(character) {
			text.WriteString(`\u{` + strconv.FormatInt(int64(character), 16) + `}`)
		} else {
			text.WriteRune(character)
		}
	}
}

// isExtendedDebugCharacter includes combining marks and the Unicode grapheme-extend property.
func isExtendedDebugCharacter(character rune) bool {
	isExtended := unicode.Is(unicode.Mn, character) || unicode.Is(unicode.Me, character)
	if table := unicode.Properties["Other_Grapheme_Extend"]; table != nil && unicode.Is(table, character) {
		isExtended = true
	}
	return isExtended
}
