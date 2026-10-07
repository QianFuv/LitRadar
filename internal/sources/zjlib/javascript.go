package zjlib

import (
	"strconv"
	"strings"
	"unicode"
	"unicode/utf8"

	urlerrors "github.com/nlnwa/whatwg-url/errors"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// decodeJsString preserves literal unknown escapes and legacy rune consumption.
func decodeJsString(value string) string {
	characters := []rune(value)
	var output strings.Builder
	for index := 0; index < len(characters); index++ {
		character := characters[index]
		if character != '\\' || index+1 == len(characters) {
			output.WriteRune(character)
			continue
		}
		index++
		index = writeJsEscape(&output, characters, index)
	}
	return output.String()
}
func quotedAssignment(text, marker string) *string {
	start := strings.Index(text, marker)
	if start < 0 {
		return nil
	}
	rest := text[start+len(marker):]
	equal := strings.IndexByte(rest, '=')
	if equal < 0 {
		return nil
	}
	rest = strings.TrimLeftFunc(rest[equal+1:], unicode.IsSpace)
	if rest == "" || rest[0] != '\'' && rest[0] != '"' {
		return nil
	}
	return pointer(decodeJsString(strings.SplitN(rest[1:], rest[:1], 2)[0]))
}
func extractJsVar(text, name string) *string { return quotedAssignment(text, "var "+name) }
func extractWindowLocation(text, base string) (string, error) {
	for _, marker := range []string{"window.location.href", "location.href", "window.location"} {
		if reference := quotedAssignment(text, marker); reference != nil {
			return joinUrl(base, *reference)
		}
	}
	return "", &Error{Kind: "Parse", Message: "Could not find JavaScript window.location redirect."}
}
func joinUrl(base, reference string) (string, error) {
	location, err := whatwg.NewParser().Parse(base)
	if err != nil {
		return "", &Error{Kind: "Parse", Message: urlParseMessage(err, false)}
	}
	joined, err := whatwg.NewParser().ParseRef(location.Href(false), reference)
	if err != nil {
		return "", &Error{Kind: "Parse", Message: urlParseMessage(err, location.OpaquePath())}
	}
	return joined.Href(false), nil
}
func urlParseMessage(err error, opaqueBase bool) string {
	switch urlerrors.Type(err) {
	case urlerrors.HostMissing:
		return "empty host"
	case urlerrors.PortInvalid, urlerrors.PortOutOfRange:
		return "invalid port number"
	case urlerrors.HostInvalidCodePoint:
		return "invalid domain character"
	case urlerrors.IPv4TooManyParts, urlerrors.IPv4NonNumericPart, urlerrors.IPv4OutOfRangePart:
		return "invalid IPv4 address"
	case urlerrors.IPv6Unclosed, urlerrors.IPv6InvalidCompression, urlerrors.IPv6TooManyPieces, urlerrors.IPv6MultipleCompression, urlerrors.IPv6InvalidCodePoint, urlerrors.IPv6TooFewPieces, urlerrors.IPv4InIPv6TooManyPieces, urlerrors.IPv4InIPv6InvalidCodePoint, urlerrors.IPv4InIPv6OutOfRangePart, urlerrors.IPv4InIPv6TooFewParts:
		return "invalid IPv6 address"
	case urlerrors.MissingSchemeNonRelativeURL:
		if opaqueBase {
			return "relative URL with a cannot-be-a-base base"
		}
		return "relative URL without a base"
	default:
		return "invalid international domain name"
	}
}

type shareCookieSync struct {
	url    string
	fields map[string]string
}

// safeAbsolutePath admits the original ASCII path grammar without dot segments.
func safeAbsolutePath(path string) bool {
	if !strings.HasPrefix(path, "/") || strings.HasPrefix(path, "//") {
		return false
	}
	for _, character := range []byte(path) {
		if !isSafePathCharacter(character) {
			return false
		}
	}
	for _, part := range strings.Split(path, "/") {
		if part == "." || part == ".." {
			return false
		}
	}
	return true
}

// extractShareCookieSync admits the callback before the domain and portal endpoint.
func extractShareCookieSync(text string, allowed endpoints) (*shareCookieSync, error) {
	sign, callback := extractJsVar(text, "sign"), extractJsVar(text, "url")
	if sign == nil || callback == nil || !strings.Contains(text, "sso-login/cookie/sync") {
		return nil, nil
	}
	base, _ := whatwg.NewParser().Parse(allowed.bases[shareFamily])
	origin := base.Scheme() + "://" + base.Host()
	normalized, portal := shareSyncDomainAndPortal(text, origin, base.Scheme())
	callbackUrl, err := allowed.join(allowed.bases[shareFamily], *callback, shareFamily)
	if err != nil {
		return nil, err
	}
	domainUrl, err := whatwg.NewParser().Parse(normalized)
	if err != nil {
		return nil, &Error{Kind: "Parse", Message: "Share cookie sync domain was invalid."}
	}
	if !isAllowedShareSync(domainUrl, base, portal) {
		return nil, &Error{Kind: "Parse", Message: "Share cookie sync endpoint was not allowed."}
	}
	syncUrl, err := allowed.parse(origin+strings.TrimRight(portal, "/")+"/sso-login/cookie/sync", shareFamily)
	if err != nil {
		return nil, err
	}
	return &shareCookieSync{url: syncUrl.Href(false), fields: map[string]string{"sign": *sign, "url": callbackUrl}}, nil
}

// writeJsEscape writes one escaped rune and returns the last consumed rune index.
func writeJsEscape(output *strings.Builder, characters []rune, index int) int {
	escaped := characters[index]
	switch escaped {
	case '/', '"', '\'', '\\':
		output.WriteRune(escaped)
	case 'b':
		output.WriteByte('\b')
	case 'f':
		output.WriteByte('\f')
	case 'n':
		output.WriteByte('\n')
	case 'r':
		output.WriteByte('\r')
	case 't':
		output.WriteByte('\t')
	case 'u':
		return writeJsUnicodeEscape(output, characters, index)
	default:
		output.WriteByte('\\')
		output.WriteRune(escaped)
	}
	return index
}

// writeJsUnicodeEscape consumes at most four runes and retains invalid escape spelling.
func writeJsUnicodeEscape(output *strings.Builder, characters []rune, index int) int {
	end := min(index+5, len(characters))
	digits := string(characters[index+1 : end])
	index = end - 1
	if len(digits) == 4 {
		number, err := strconv.ParseUint(strings.TrimPrefix(digits, "+"), 16, 32)
		if err == nil && number <= utf8.MaxRune && (number < 0xd800 || number > 0xdfff) {
			output.WriteRune(rune(number))
			return index
		}
	}
	output.WriteString(`\u`)
	output.WriteString(digits)
	return index
}

// isSafePathCharacter preserves the ASCII portal path alphabet.
func isSafePathCharacter(character byte) bool {
	return character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("/-_.", rune(character))
}

// isAllowedShareSync checks origin and root domain before the absolute portal path.
func isAllowedShareSync(domainUrl, base *whatwg.Url, portal string) bool {
	return !(!hasOrigin(domainUrl, base) || strings.Trim(domainUrl.Pathname(), "/") != "" || strings.Contains(domainUrl.Href(true), "?") || domainUrl.Href(false) != domainUrl.Href(true) || !safeAbsolutePath(portal))
}

// shareSyncDomainAndPortal preserves missing defaults and protocol-relative domain normalization.
func shareSyncDomainAndPortal(text, origin, scheme string) (string, string) {
	domain, portal := extractJsVar(text, "domainUrl"), extractJsVar(text, "portalContextPath")
	if domain == nil {
		domain = &origin
	}
	if portal == nil {
		portal = pointer("/entry")
	}
	normalized := *domain
	if strings.HasPrefix(normalized, "//") {
		normalized = scheme + ":" + normalized
	}
	return normalized, *portal
}
