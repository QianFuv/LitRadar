package scholarly

import (
	"encoding/json"
	"errors"
	"strconv"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/sources"
)

// NormalizeDoi preserves the permissive source-layer DOI normalization.
func NormalizeDoi(value any) *string {
	if value == nil {
		return nil
	}
	text, isString := value.(string)
	if !isString {
		encoded, err := domain.Json(value)
		if err != nil {
			return nil
		}
		text = string(encoded)
	}
	text = domain.Lowercase(strings.TrimSpace(text))
	for _, prefix := range []string{"https://doi.org/", "http://doi.org/", "doi:"} {
		if strings.HasPrefix(text, prefix) {
			text = text[len(prefix):]
			break
		}
	}
	text = strings.TrimSpace(text)
	if text == "" {
		return nil
	}
	return &text
}

func uniqueDois(values []string) []string {
	result := []string{}
	seen := map[string]bool{}
	for _, value := range values {
		if normalized := NormalizeDoi(value); normalized != nil && !seen[*normalized] {
			seen[*normalized] = true
			result = append(result, *normalized)
		}
	}
	return result
}

// ValuePool splits configured credential pools, retaining first occurrence order.
func ValuePool(value string) []string {
	result := []string{}
	seen := map[string]bool{}
	for _, part := range strings.FieldsFunc(value, func(character rune) bool { return character == ',' || character == ';' || character == '\n' }) {
		part = strings.TrimSpace(part)
		if part != "" && !seen[part] {
			seen[part] = true
			result = append(result, part)
		}
	}
	return result
}

// RedactUrl removes sensitive query values without reparsing opaque fixture URLs.
func RedactUrl(value string) string {
	base, query, hasQuery := strings.Cut(value, "?")
	if !hasQuery {
		return value
	}
	parts := strings.Split(query, "&")
	for index, part := range parts {
		key, _, _ := strings.Cut(part, "=")
		switch key {
		case "api_key", "x-api-key":
			parts[index] = key + "=SECRET"
		case "filter", "search", "mailto", "cursor":
			parts[index] = key + "=REDACTED"
		}
	}
	return base + "?" + strings.Join(parts, "&")
}

// QueryPair preserves upstream query ordering.
type QueryPair struct{ Name, Value string }

// EncodeQuery follows application/x-www-form-urlencoded, including '*' and '~'.
func EncodeQuery(pairs []QueryPair) string {
	parts := make([]string, len(pairs))
	for index, pair := range pairs {
		parts[index] = formEncode(pair.Name) + "=" + formEncode(pair.Value)
	}
	return strings.Join(parts, "&")
}

func formEncode(value string) string {
	const hex = "0123456789ABCDEF"
	var output strings.Builder
	for index := range len(value) {
		character := value[index]
		switch {
		case character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("*-._", rune(character)):
			output.WriteByte(character)
		case character == ' ':
			output.WriteByte('+')
		default:
			output.WriteByte('%')
			output.WriteByte(hex[character>>4])
			output.WriteByte(hex[character&15])
		}
	}
	return output.String()
}

// OpenAlexDoiQuery creates the ordered DOI filter query with an optional credential.
func OpenAlexDoiQuery(dois []string, apiKey *string) []QueryPair {
	pairs := []QueryPair{{"filter", "doi:" + strings.Join(dois, "|")}, {"per-page", strconv.Itoa(max(len(dois), 1))}, {"select", OpenAlexDoiFields}}
	if apiKey != nil {
		pairs = append(pairs, QueryPair{"api_key", *apiKey})
	}
	return pairs
}

// PartitionOpenAlexDois enforces both the upstream DOI count and encoded URL limits.
func PartitionOpenAlexDois(dois []string, batchSize int, apiKey *string) ([][]string, error) {
	batches := [][]string{}
	batch := []string{}
	batchSize = max(1, min(batchSize, 100))
	for _, doi := range dois {
		candidate := append(append([]string{}, batch...), doi)
		if len(candidate) > batchSize || len("https://api.openalex.org/works?"+EncodeQuery(OpenAlexDoiQuery(candidate, apiKey))) > 1900 {
			if len(batch) > 0 {
				batches = append(batches, batch)
			}
			batch = []string{doi}
			if len("https://api.openalex.org/works?"+EncodeQuery(OpenAlexDoiQuery(batch, apiKey))) > 1900 {
				return nil, &Error{Kind: "Configuration", Message: "One OpenAlex DOI enrichment value exceeds the request URL budget."}
			}
		} else {
			batch = candidate
		}
	}
	if len(batch) > 0 {
		batches = append(batches, batch)
	}
	return batches, nil
}

// CrossrefParameters validates frozen date bounds and builds the canonical query.
func CrossrefParameters(query CrossrefQuery) ([]QueryPair, error) {
	filters := []string{"type:journal-article"}
	pairs := []QueryPair{}
	if query.IsEarliest {
		until, err := crossrefTimestamp(query.CreatedUntil)
		if err != nil {
			return nil, err
		}
		filters = append(filters, "until-created-date:"+until)
		pairs = append(pairs, QueryPair{"rows", "1"}, QueryPair{"sort", "created"}, QueryPair{"order", "asc"})
	} else {
		if query.CreatedFrom > query.CreatedUntil || (query.UpdatedFrom == nil) != (query.UpdatedUntil == nil) {
			return nil, &Error{Kind: "Configuration", Message: "invalid Crossref date bounds"}
		}
		from, err := crossrefTimestamp(query.CreatedFrom)
		if err != nil {
			return nil, err
		}
		until, err := crossrefTimestamp(query.CreatedUntil)
		if err != nil {
			return nil, err
		}
		filters = append(filters, "from-created-date:"+from, "until-created-date:"+until)
		if query.UpdatedFrom != nil {
			updated, err := crossrefTimestamp(*query.UpdatedUntil)
			if err != nil {
				return nil, err
			}
			filters = append(filters, "from-update-date:"+*query.UpdatedFrom, "until-update-date:"+updated)
		}
		pairs = append(pairs, QueryPair{"rows", "225"})
		if query.Cursor != nil {
			pairs = append(pairs, QueryPair{"cursor", *query.Cursor})
		}
	}
	return append(pairs, QueryPair{"filter", strings.Join(filters, ",")}), nil
}

func sourceNumber(value any) (domain.Number, bool) {
	var token json.Number
	switch item := value.(type) {
	case domain.Number:
		return item, true
	case json.Number:
		token = item
	case int:
		token = json.Number(strconv.Itoa(item))
	case int64:
		token = json.Number(strconv.FormatInt(item, 10))
	case uint64:
		token = json.Number(strconv.FormatUint(item, 10))
	default:
		return domain.Number{}, false
	}
	parsed, err := domain.ParseNumber(token)
	return parsed, err == nil
}

func crossrefTimestamp(value int64) (string, error) {
	if value < -8334601228800 || value > 8210266876799 {
		return "", &Error{Kind: "Configuration", Message: "Crossref timestamp is out of range"}
	}
	date := time.Unix(value, 0).UTC()
	year := date.Year()
	prefix := strconv.Itoa(year)
	if year >= 0 && year <= 9999 {
		prefix = strings.Repeat("0", 4-len(prefix)) + prefix
	} else if year > 9999 {
		prefix = "+" + prefix
	} else if year > -1000 {
		digits := strconv.Itoa(-year)
		prefix = "-" + strings.Repeat("0", 4-len(digits)) + digits
	}
	return prefix + date.Format("-01-02T15:04:05"), nil
}

func field(value any, key string) any { object, _ := value.(map[string]any); return object[key] }
func array(value any) []any {
	items, ok := value.([]any)
	if !ok {
		return []any{}
	}
	return items
}
func optionalString(value any) *string {
	text, ok := value.(string)
	if !ok {
		return nil
	}
	return &text
}
func normalizedTitle(value string) string {
	return domain.Lowercase(strings.Join(strings.Fields(value), " "))
}
func normalizedIssn(value string) string {
	value = strings.ToUpper(strings.ReplaceAll(strings.TrimSpace(value), "-", ""))
	if len(value) != 8 {
		return ""
	}
	for index, character := range []byte(value) {
		if character < '0' || character > '9' {
			if index != 7 || character != 'X' {
				return ""
			}
		}
	}
	return value
}
func sourceMatchesIssn(value any, target string) bool {
	target = normalizedIssn(target)
	if target == "" {
		return false
	}
	candidates := append([]any{field(value, "issn_l")}, array(field(value, "issn"))...)
	for _, candidate := range candidates {
		if text, ok := candidate.(string); ok && normalizedIssn(text) == target {
			return true
		}
	}
	return false
}
func isPlanRestriction(err error) bool {
	var failure *Error
	if !errors.As(err, &failure) || failure.Kind != "HttpStatus" || failure.Service != OpenAlex || failure.Endpoint != "source_works" || failure.StatusCode != 429 {
		return false
	}
	message, _ := field(failure.Body, "message").(string)
	return field(failure.Body, "error") == "Plan upgrade required" && strings.Contains(message, "from_created_date")
}
func isNoValidIds(err error) bool {
	var failure *Error
	if !errors.As(err, &failure) || failure.Kind != "HttpStatus" || failure.Service != SemanticScholar || failure.Endpoint != "paper_batch" || failure.StatusCode != 400 {
		return false
	}
	message, ok := field(failure.Body, "error").(string)
	if !ok {
		return false
	}
	message = strings.TrimSpace(message)
	for _, character := range message {
		if character > 127 {
			return false
		}
	}
	return strings.EqualFold(message, "no valid paper ids given")
}
