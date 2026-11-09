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
	defaultPort := proxyDefaultPort(location.Scheme())
	if defaultPort == "" {
		return "", invalid
	}
	hasUserinfo := strings.Contains(rawAuthority(value), "@")
	if !isProxyLocation(value, declared, location, hasUserinfo) {
		return "", invalid
	}
	if hasUserinfo && !isProxyUserinfo(location) {
		return "", invalid
	}
	location.SetHostname(asciiLower(location.Hostname()))
	if location.Port() == "" {
		location.SetPort(defaultPort)
	}
	location.SetPathname("")
	return location.Href(false), nil
}

// proxyDefaultPort preserves the supported proxy schemes and their default ports.
func proxyDefaultPort(scheme string) string {
	switch scheme {
	case "http":
		return "80"
	case "https":
		return "443"
	case "socks5", "socks5h":
		return "1080"
	default:
		return ""
	}
}

// isProxyLocation checks explicit authority and complete credentials before endpoint syntax.
func isProxyLocation(value, declared string, location *whatwg.Url, hasUserinfo bool) bool {
	hasCompleteUserinfo := location.Username() != "" && location.Password() != ""
	return asciiLower(declared) == location.Scheme() && rawAuthority(value) != "" && location.Hostname() != "" && hasUserinfo == hasCompleteUserinfo && isProxyEndpoint(location)
}

// isProxyEndpoint restricts a proxy URL to an unadorned root endpoint and nonzero port.
func isProxyEndpoint(location *whatwg.Url) bool {
	return location.Port() != "0" && (location.Pathname() == "" || location.Pathname() == "/") && !hasQueryOrFragment(location) && !location.OpaquePath()
}

// isProxyUserinfo validates percent escapes and SOCKS credential byte lengths.
func isProxyUserinfo(location *whatwg.Url) bool {
	usernameLength, isUsernameValid := percentDecodedLength(location.Username())
	passwordLength, isPasswordValid := percentDecodedLength(location.Password())
	return isUsernameValid && isPasswordValid && (!strings.HasPrefix(location.Scheme(), "socks5") || usernameLength <= 255 && passwordLength <= 255)
}

// CanonicalizeBaseUrl preserves the exact administrator-approved HTTPS endpoint boundary.
func CanonicalizeBaseUrl(value string) (string, error) {
	invalid := errors.New("AI allowed base URLs must be exact HTTPS base URLs without credentials, query, fragment, or port zero")
	value = strings.TrimSpace(value)
	location, err := whatwg.NewParser().Parse(value)
	if err != nil || !isBaseUrlLocation(value, location) {
		return "", invalid
	}
	result := location.Href(false)
	if !strings.HasSuffix(location.Pathname(), "/") {
		result += "/"
	}
	return result, nil
}

// isBaseUrlLocation preserves the exact credential-free administrator endpoint boundary.
func isBaseUrlLocation(value string, location *whatwg.Url) bool {
	return location.Scheme() == "https" && location.Hostname() != "" && !strings.Contains(rawAuthority(value), "@") && location.Username() == "" && location.Password() == "" && location.Port() != "0" && !hasQueryOrFragment(location) && !location.OpaquePath()
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
