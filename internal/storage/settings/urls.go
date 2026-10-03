package settings

import (
	"errors"
	"strings"

	whatwg "github.com/nlnwa/whatwg-url/url"
)

func rawAuthority(value string) string {
	_, remainder, found := strings.Cut(value, "://")
	if !found {
		return ""
	}
	if index := strings.IndexAny(remainder, "/?#"); index >= 0 {
		return remainder[:index]
	}
	return remainder
}
func hasQueryOrFragment(location *whatwg.Url) bool {
	return strings.Contains(location.Href(true), "?") || location.Href(false) != location.Href(true)
}

func normalizeProxyUrl(value string) (string, error) {
	invalid := errors.New("Invalid Provider proxy URL")
	value = strings.TrimSpace(value)
	if value == "" {
		return "", nil
	}
	declared, _, hasScheme := strings.Cut(value, "://")
	if !hasScheme {
		return "", invalid
	}
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return "", invalid
	}
	defaultPort := ""
	switch location.Scheme() {
	case "http":
		defaultPort = "80"
	case "https":
		defaultPort = "443"
	case "socks5", "socks5h":
		defaultPort = "1080"
	default:
		return "", invalid
	}
	hasUserinfo := strings.Contains(rawAuthority(value), "@")
	hasCompleteUserinfo := location.Username() != "" && location.Password() != ""
	if asciiLower(declared) != location.Scheme() || rawAuthority(value) == "" || location.Hostname() == "" || hasUserinfo != hasCompleteUserinfo || location.Port() == "0" || (location.Pathname() != "" && location.Pathname() != "/") || hasQueryOrFragment(location) || location.OpaquePath() {
		return "", invalid
	}
	if hasUserinfo {
		usernameLength, isUsernameValid := percentDecodedLength(location.Username())
		passwordLength, isPasswordValid := percentDecodedLength(location.Password())
		if !isUsernameValid || !isPasswordValid || (strings.HasPrefix(location.Scheme(), "socks5") && (usernameLength > 255 || passwordLength > 255)) {
			return "", invalid
		}
	}
	location.SetHostname(asciiLower(location.Hostname()))
	if location.Port() == "" {
		location.SetPort(defaultPort)
	}
	location.SetPathname("")
	return location.Href(false), nil
}

// CanonicalizeBaseUrl preserves the exact administrator-approved HTTPS endpoint boundary.
func CanonicalizeBaseUrl(value string) (string, error) {
	invalid := errors.New("AI allowed base URLs must be exact HTTPS base URLs without credentials, query, fragment, or port zero")
	value = strings.TrimSpace(value)
	location, err := whatwg.NewParser().Parse(value)
	if err != nil || location.Scheme() != "https" || location.Hostname() == "" || strings.Contains(rawAuthority(value), "@") || location.Username() != "" || location.Password() != "" || location.Port() == "0" || hasQueryOrFragment(location) || location.OpaquePath() {
		return "", invalid
	}
	result := location.Href(false)
	if !strings.HasSuffix(location.Pathname(), "/") {
		result += "/"
	}
	return result, nil
}

func percentDecodedLength(value string) (int, bool) {
	length := 0
	isHex := func(character byte) bool {
		return character >= '0' && character <= '9' || character >= 'a' && character <= 'f' || character >= 'A' && character <= 'F'
	}
	for index := 0; index < len(value); index++ {
		if value[index] == '%' {
			if index+2 >= len(value) || !isHex(value[index+1]) || !isHex(value[index+2]) {
				return 0, false
			}
			index += 2
		}
		length++
	}
	return length, true
}
