package cnki

import (
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

func parseDomesticUrl(value string) (*whatwg.Url, error) {
	value = strings.TrimSpace(value)
	if value == "" || strings.HasPrefix(value, "//") {
		return nil, &Error{Kind: "Request", Message: "domestic CNKI URL is invalid"}
	}
	parser := whatwg.NewParser()
	parsed, err := parser.Parse(value)
	if err != nil {
		if !strings.HasPrefix(value, "/") {
			return nil, &Error{Kind: "Request", Message: "domestic CNKI relative URL is invalid"}
		}
		base := NaviBase
		if strings.HasPrefix(value, "/kcms") || strings.HasPrefix(value, "/starter") || strings.HasPrefix(value, "/verify") {
			base = KnsBase
		}
		parsed, err = parser.ParseRef(base, value)
		if err != nil {
			return nil, &Error{Kind: "Request", Message: "domestic CNKI URL is invalid"}
		}
	}
	if parsed.Scheme() != "https" || parsed.Hostname() != "navi.cnki.net" && parsed.Hostname() != "kns.cnki.net" || parsed.Username() != "" || parsed.Password() != "" || parsed.Port() != "" && parsed.Port() != "443" {
		return nil, &Error{Kind: "Request", Message: "domestic CNKI URL is not allowed"}
	}
	return parsed, nil
}

func formDecode(value string) string {
	value = strings.ReplaceAll(value, "+", " ")
	output := make([]byte, 0, len(value))
	for index := 0; index < len(value); index++ {
		if value[index] == '%' && index+2 < len(value) {
			if character, err := strconv.ParseUint(value[index+1:index+3], 16, 8); err == nil {
				output = append(output, byte(character))
				index += 2
				continue
			}
		}
		output = append(output, value[index])
	}
	return transport.LossyUtf8(output)
}
func queryPairs(query string) []scholarly.QueryPair {
	pairs := []scholarly.QueryPair{}
	for _, part := range strings.Split(strings.TrimPrefix(query, "?"), "&") {
		if part == "" {
			continue
		}
		key, value, _ := strings.Cut(part, "=")
		pairs = append(pairs, scholarly.QueryPair{Name: formDecode(key), Value: formDecode(value)})
	}
	return pairs
}
func asciiLower(value string) string {
	var output strings.Builder
	for index := range len(value) {
		character := value[index]
		if character >= 'A' && character <= 'Z' {
			character += 'a' - 'A'
		}
		output.WriteByte(character)
	}
	return output.String()
}

// WithDomesticPlatform validates a domestic URL and replaces all platform query aliases.
func WithDomesticPlatform(value string) (string, error) {
	parsed, err := parseDomesticUrl(value)
	if err != nil {
		return "", err
	}
	pairs := []scholarly.QueryPair{}
	for _, pair := range queryPairs(parsed.Search()) {
		key := asciiLower(pair.Name)
		if key != "language" && key != "uniplatform" {
			pairs = append(pairs, pair)
		}
	}
	pairs = append(pairs, scholarly.QueryPair{Name: "uniplatform", Value: "NZKPT"}, scholarly.QueryPair{Name: "language", Value: "CHS"})
	parsed.SetSearch(scholarly.EncodeQuery(pairs))
	return parsed.Href(false), nil
}

// RedactUrl retains nonsensitive query order and removes challenge credentials.
func RedactUrl(value string) string {
	parsed, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return "[REDACTED INVALID DOMESTIC URL]"
	}
	pairs := queryPairs(parsed.Search())
	for index, pair := range pairs {
		switch asciiLower(pair.Name) {
		case "captchaid", "ident", "returnurl", "token", "secretkey", "pointjson", "originalimagebase64", "jigsawimagebase64":
			pairs[index].Value = "[REDACTED]"
		}
	}
	parsed.SetSearch("")
	if len(pairs) > 0 {
		parsed.SetSearch(scholarly.EncodeQuery(pairs))
	}
	return parsed.Href(false)
}

// ContainsOverseasHost identifies forbidden overseas references in URLs or response bodies.
func ContainsOverseasHost(value string) bool {
	return strings.Contains(strings.ToLower(value), "oversea.cnki.net")
}
