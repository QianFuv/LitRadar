package mcpcompat

import (
	"io"
	"net/http"
	"strconv"
	"strings"
)

// HostOriginPolicy preserves the configured legacy DNS-rebinding checks.
// An empty host list permits valid authorities; an empty origin list skips origin checks.
type HostOriginPolicy struct {
	AllowedHosts   []string
	AllowedOrigins []string
}

type authority struct {
	host string
	port int
}
type origin struct {
	scheme    string
	authority authority
}

func asciiLower(value string) string {
	return strings.Map(func(value rune) rune {
		if value >= 'A' && value <= 'Z' {
			return value + 32
		}
		return value
	}, value)
}

func normalizeHost(value string) string {
	return asciiLower(strings.Trim(strings.Trim(value, "["), "]"))
}

// parseAuthority preserves structural authority validation and legacy port extraction.
func parseAuthority(value string) (authority, bool) {
	if value == "" {
		return authority{}, false
	}
	atSign, ok := authorityUserinfoBoundary(value)
	if !ok {
		return authority{}, false
	}
	hostPort := value[atSign+1:]
	host := hostPort
	if strings.HasPrefix(hostPort, "[") {
		host = hostPort[:strings.IndexByte(hostPort, ']')+1]
	} else if index := strings.IndexByte(hostPort, ':'); index >= 0 {
		host = hostPort[:index]
	}
	port := -1
	if index := strings.LastIndexByte(value, ':'); index >= 0 {
		if number, err := strconv.ParseUint(strings.TrimPrefix(value[index+1:], "+"), 10, 16); err == nil {
			port = int(number)
		}
	}
	return authority{host: normalizeHost(host), port: port}, true
}

// parseOrigin validates legacy scheme and tail syntax before authority parsing.
func parseOrigin(value string) (origin, bool) {
	value = strings.TrimSpace(value)
	if strings.EqualFold(value, "null") {
		return origin{scheme: "null"}, true
	}
	if len(value) > 65534 {
		return origin{}, false
	}
	scheme, remainder, found := strings.Cut(value, "://")
	if !found || scheme == "" || len(scheme) > 64 {
		return origin{}, false
	}
	if !validOriginScheme(scheme) {
		return origin{}, false
	}
	if index := strings.IndexAny(remainder, "/?#"); index >= 0 {
		if !validOriginTail(remainder[index:]) {
			return origin{}, false
		}
		remainder = remainder[:index]
	}
	parsed, ok := parseAuthority(remainder)
	return origin{scheme: asciiLower(scheme), authority: parsed}, ok
}

func visibleHeader(value string) bool {
	for _, character := range []byte(value) {
		if character != '\t' && (character < 32 || character >= 127) {
			return false
		}
	}
	return true
}

func headerError(writer http.ResponseWriter, status int, message string) bool {
	if status == 400 {
		writer.Header().Set("Content-Type", "text/plain; charset=utf-8")
	} else {
		writer.Header()["Content-Type"] = nil
	}
	writer.WriteHeader(status)
	io.WriteString(writer, message)
	return false
}

// validate checks Host admission before inspecting the first Origin header.
func (policy HostOriginPolicy) validate(writer http.ResponseWriter, request *http.Request) bool {
	host, hasHost := requestAuthority(request)
	if !hasHost {
		return headerError(writer, 400, "Bad Request: missing Host header")
	}
	if !visibleHeader(host) {
		return headerError(writer, 400, "Bad Request: Invalid Host header encoding")
	}
	parsed, ok := parseAuthority(host)
	if !ok {
		return headerError(writer, 400, "Bad Request: Invalid Host header")
	}
	if !policy.allowsAuthority(parsed) {
		return headerError(writer, 403, "Forbidden: Host header is not allowed")
	}
	return policy.validateOrigin(writer, request)
}

// authoritySyntax retains global bracket state and local colon/percent state across userinfo.
type authoritySyntax struct {
	colons     int
	atSign     int
	isOpened   bool
	isClosed   bool
	hasPercent bool
}

// authorityUserinfoBoundary validates bytes and returns the last userinfo separator.
func authorityUserinfoBoundary(value string) (int, bool) {
	state := authoritySyntax{atSign: -1}
	for index, character := range []byte(value) {
		if !state.accept(character, index) {
			return 0, false
		}
	}
	if state.isOpened != state.isClosed || state.colons > 1 || state.atSign == len(value)-1 || state.hasPercent {
		return 0, false
	}
	return state.atSign, true
}

// accept updates the authority syntax state using the original ASCII allowlist.
func (state *authoritySyntax) accept(character byte, index int) bool {
	if !strings.ContainsRune("!$&'()*+,-.0123456789:;=@ABCDEFGHIJKLMNOPQRSTUVWXYZ[]_abcdefghijklmnopqrstuvwxyz~%", rune(character)) {
		return false
	}
	switch character {
	case '%':
		state.hasPercent = true
	case ':':
		state.colons++
		return state.colons <= 8
	case '[':
		return state.openBracket()
	case ']':
		return state.closeBracket()
	case '@':
		state.atSign = index
		state.colons = 0
		state.hasPercent = false
	}
	return true
}

// openBracket rejects repeated brackets and an active percent marker.
func (state *authoritySyntax) openBracket() bool {
	if state.hasPercent || state.isOpened {
		return false
	}
	state.isOpened = true
	return true
}

// closeBracket preserves bracket state while clearing local colon and percent counters.
func (state *authoritySyntax) closeBracket() bool {
	if !state.isOpened || state.isClosed {
		return false
	}
	state.isClosed = true
	state.colons = 0
	state.hasPercent = false
	return true
}

// validOriginScheme retains the legacy digit and punctuation allowance.
func validOriginScheme(value string) bool {
	for _, character := range []byte(value) {
		if !strings.ContainsRune("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+.-~", rune(character)) {
			return false
		}
	}
	return true
}

// validOriginTail validates path and query bytes up to the first ignored fragment.
func validOriginTail(value string) bool {
	isQuery := false
	for _, character := range []byte(value) {
		if character == '#' {
			break
		}
		if character == '?' {
			isQuery = true
			continue
		}
		if !validOriginTailCharacter(character, isQuery) {
			return false
		}
	}
	return true
}

// validOriginTailCharacter preserves the distinct legacy path and query allowlists.
func validOriginTailCharacter(character byte, isQuery bool) bool {
	isAllowed := character == 0x21 || character >= 0x24 && character <= 0x3b || character == 0x3d
	if isQuery {
		return isAllowed || character >= 0x3f && character <= 0x7e
	}
	return isAllowed || validOriginPathCharacter(character)
}

// validOriginPathCharacter admits the path-only byte ranges and legacy punctuation.
func validOriginPathCharacter(character byte) bool {
	return character >= 0x40 && character <= 0x5f || character >= 0x61 && character <= 0x7a || strings.ContainsRune("|~\"{}", rune(character))
}

// requestAuthority retains present-empty header precedence and URL fallback.
func requestAuthority(request *http.Request) (string, bool) {
	host := request.Host
	hasHost := host != ""
	if values, exists := request.Header["Host"]; exists && len(values) > 0 {
		host = values[0]
		hasHost = true
	} else if host == "" {
		host = request.URL.Host
		hasHost = host != ""
	}
	return host, hasHost
}

// allowsAuthority retains wildcard ports and normalization fallback for configured hosts.
func (policy HostOriginPolicy) allowsAuthority(parsed authority) bool {
	allowed := len(policy.AllowedHosts) == 0
	for _, entry := range policy.AllowedHosts {
		entry = strings.TrimSpace(entry)
		if entry == "" {
			continue
		}
		candidate, ok := parseAuthority(entry)
		if !ok {
			candidate = authority{host: normalizeHost(entry), port: -1}
		}
		if candidate.host == parsed.host && (candidate.port < 0 || candidate.port == parsed.port) {
			allowed = true
			break
		}
	}
	return allowed
}

// validateOrigin skips parsing when policy or the header is absent.
func (policy HostOriginPolicy) validateOrigin(writer http.ResponseWriter, request *http.Request) bool {
	values := request.Header.Values("Origin")
	if len(policy.AllowedOrigins) == 0 || len(values) == 0 {
		return true
	}
	if !visibleHeader(values[0]) {
		return headerError(writer, 400, "Bad Request: Invalid Origin header encoding")
	}
	actual, ok := parseOrigin(values[0])
	if !ok {
		return headerError(writer, 400, "Bad Request: Invalid Origin header")
	}
	if policy.allowsOrigin(actual) {
		return true
	}
	return headerError(writer, 403, "Forbidden: Origin header is not allowed")
}

// allowsOrigin compares only successfully parsed configured origins.
func (policy HostOriginPolicy) allowsOrigin(actual origin) bool {
	for _, entry := range policy.AllowedOrigins {
		candidate, ok := parseOrigin(entry)
		if ok && candidate.scheme == actual.scheme && candidate.authority.host == actual.authority.host && (candidate.authority.port < 0 || candidate.authority.port == actual.authority.port) {
			return true
		}
	}
	return false
}
