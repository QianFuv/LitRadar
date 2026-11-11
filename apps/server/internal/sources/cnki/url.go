package cnki

import (
	"net/url"
	"strings"

	"github.com/QianFuv/LitRadar/apps/server/internal/sources/scholarly"
	"github.com/QianFuv/LitRadar/apps/server/internal/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// parseDomesticUrl admits the original domestic URL grammar after relative resolution.
func parseDomesticUrl(value string) (*whatwg.Url, error) {
	value = strings.TrimSpace(value)
	if value == "" || strings.HasPrefix(value, "//") {
		return nil, &Error{Kind: "Request", Message: "domestic CNKI URL is invalid"}
	}
	parsed, err := resolveDomesticUrl(value)
	if err != nil {
		return nil, err
	}
	if !isAllowedDomesticUrl(parsed) {
		return nil, &Error{Kind: "Request", Message: "domestic CNKI URL is not allowed"}
	}
	return parsed, nil
}

func queryPairs(query string) ([]scholarly.QueryPair, error) {
	pairs := []scholarly.QueryPair{}
	for part := range strings.SplitSeq(strings.TrimPrefix(query, "?"), "&") {
		if part == "" {
			continue
		}
		key, value, _ := strings.Cut(part, "=")
		name, keyError := url.QueryUnescape(key)
		decoded, valueError := url.QueryUnescape(value)
		if keyError != nil || valueError != nil {
			return nil, &Error{Kind: "Request", Message: "domestic CNKI URL query is invalid"}
		}
		pairs = append(pairs, scholarly.QueryPair{Name: transport.LossyUtf8([]byte(name)), Value: transport.LossyUtf8([]byte(decoded))})
	}
	return pairs, nil
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
	query, err := queryPairs(parsed.Search())
	if err != nil {
		return "", err
	}
	pairs := []scholarly.QueryPair{}
	for _, pair := range query {
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
	pairs, err := queryPairs(parsed.Search())
	if err != nil {
		return "[REDACTED INVALID DOMESTIC URL]"
	}
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

// resolveDomesticUrl preserves absolute parsing and the three relative KNS prefixes.
func resolveDomesticUrl(value string) (*whatwg.Url, error) {
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
	return parsed, nil
}

// isAllowedDomesticUrl admits only credential-free domestic HTTPS endpoints.
func isAllowedDomesticUrl(parsed *whatwg.Url) bool {
	return !(parsed.Scheme() != "https" || parsed.Hostname() != "navi.cnki.net" && parsed.Hostname() != "kns.cnki.net" || parsed.Username() != "" || parsed.Password() != "" || parsed.Port() != "" && parsed.Port() != "443")
}
