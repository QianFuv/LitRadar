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

func parseAuthority(value string) (authority, bool) {
	if value == "" {
		return authority{}, false
	}
	colons, atSign := 0, -1
	opened, closed, percent := false, false, false
	for index, character := range []byte(value) {
		if !strings.ContainsRune("!$&'()*+,-.0123456789:;=@ABCDEFGHIJKLMNOPQRSTUVWXYZ[]_abcdefghijklmnopqrstuvwxyz~%", rune(character)) {
			return authority{}, false
		}
		switch character {
		case '%':
			percent = true
		case ':':
			colons++
			if colons > 8 {
				return authority{}, false
			}
		case '[':
			if percent || opened {
				return authority{}, false
			}
			opened = true
		case ']':
			if !opened || closed {
				return authority{}, false
			}
			closed = true
			colons = 0
			percent = false
		case '@':
			atSign = index
			colons = 0
			percent = false
		}
	}
	if opened != closed || colons > 1 || atSign == len(value)-1 || percent {
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
	for _, character := range []byte(scheme) {
		if !(character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("+.-~", rune(character))) {
			return origin{}, false
		}
	}
	if index := strings.IndexAny(remainder, "/?#"); index >= 0 {
		isQuery := false
		for _, character := range []byte(remainder[index:]) {
			if character == '#' {
				break
			}
			if character == '?' {
				isQuery = true
				continue
			}
			isAllowed := character == 0x21 || character >= 0x24 && character <= 0x3b || character == 0x3d
			if isQuery {
				isAllowed = isAllowed || character >= 0x3f && character <= 0x7e
			} else {
				isAllowed = isAllowed || character >= 0x40 && character <= 0x5f || character >= 0x61 && character <= 0x7a || strings.ContainsRune("|~\"{}", rune(character))
			}
			if !isAllowed {
				return origin{}, false
			}
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

func (policy HostOriginPolicy) validate(writer http.ResponseWriter, request *http.Request) bool {
	host := request.Host
	hasHost := host != ""
	if values, exists := request.Header["Host"]; exists && len(values) > 0 {
		host = values[0]
		hasHost = true
	} else if host == "" {
		host = request.URL.Host
		hasHost = host != ""
	}
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
	if !allowed {
		return headerError(writer, 403, "Forbidden: Host header is not allowed")
	}
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
	for _, entry := range policy.AllowedOrigins {
		candidate, ok := parseOrigin(entry)
		if ok && candidate.scheme == actual.scheme && candidate.authority.host == actual.authority.host && (candidate.authority.port < 0 || candidate.authority.port == actual.authority.port) {
			return true
		}
	}
	return headerError(writer, 403, "Forbidden: Origin header is not allowed")
}
