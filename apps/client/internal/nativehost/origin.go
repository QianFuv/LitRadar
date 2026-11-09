// Package nativehost implements the Wails services of the LitRadar desktop and
// mobile hosts.
//
// The [Bridge] service owns every request to the LitRadar server: it restricts
// traffic to the configured server origin and API paths, attaches the bearer
// token read from OS secure storage, and keeps that token away from JavaScript.
// It also stores the server origin, and hands downloads and external links to
// the operating system.
package nativehost

import (
	"net"
	"net/url"
	"path"
	"strings"
)

// readinessPath is the only non-API path the bridge may request; the server
// setup probe uses it to confirm that an origin is a LitRadar server.
const readinessPath = "/health/ready"

// apiPathPrefix is the namespace of the LitRadar API.
const apiPathPrefix = "/api/"

// NormalizeServerOrigin validates a user-entered server address and returns its
// canonical origin: a lowercase scheme and host without a default port, path,
// query, fragment, or credentials. https:// is accepted for any host; http:// is
// accepted only for loopback hosts so a development server can be used.
func NormalizeServerOrigin(raw string) (string, error) {
	trimmed := strings.TrimSpace(raw)
	parsed, err := url.Parse(trimmed)
	if err != nil || trimmed == "" {
		return "", newBridgeError(CodeInvalidOrigin, "server address is not a valid URL")
	}
	if parsed.User != nil || parsed.RawQuery != "" || parsed.Fragment != "" || parsed.Opaque != "" {
		return "", newBridgeError(CodeInvalidOrigin, "server address must not contain credentials, a query, or a fragment")
	}
	if parsed.Path != "" && parsed.Path != "/" {
		return "", newBridgeError(CodeInvalidOrigin, "server address must not contain a path")
	}
	origin, err := canonicalOrigin(parsed)
	if err != nil {
		return "", err
	}
	return origin, nil
}

// canonicalOrigin returns the canonical origin of an absolute URL and enforces
// the scheme policy of [NormalizeServerOrigin].
func canonicalOrigin(parsed *url.URL) (string, error) {
	scheme := strings.ToLower(parsed.Scheme)
	hostname := strings.ToLower(parsed.Hostname())
	if hostname == "" {
		return "", newBridgeError(CodeInvalidOrigin, "server address has no host")
	}
	switch scheme {
	case "https":
	case "http":
		if !isLoopbackHost(hostname) {
			return "", newBridgeError(CodeInsecureOrigin, "http is allowed only for loopback hosts")
		}
	default:
		return "", newBridgeError(CodeInvalidOrigin, "server address must use https")
	}
	port := parsed.Port()
	if (scheme == "https" && port == "443") || (scheme == "http" && port == "80") {
		port = ""
	}
	host := hostname
	if strings.Contains(hostname, ":") {
		host = "[" + hostname + "]"
	}
	if port != "" {
		host += ":" + port
	}
	return scheme + "://" + host, nil
}

// isLoopbackHost reports whether hostname names the local machine.
func isLoopbackHost(hostname string) bool {
	if hostname == "localhost" {
		return true
	}
	address := net.ParseIP(hostname)
	return address != nil && address.IsLoopback()
}

// resolveServerURL validates that rawURL targets origin and an allowed path and
// returns the parsed URL. Allowed paths are the API namespace and the readiness
// probe; dot segments and encoded separators are rejected so a request cannot
// escape the namespace after server-side path normalization.
func resolveServerURL(origin, rawURL string) (*url.URL, error) {
	parsed, err := url.Parse(rawURL)
	if err != nil || !parsed.IsAbs() || parsed.User != nil || parsed.Opaque != "" {
		return nil, newBridgeError(CodeForbiddenTarget, "request URL is not an absolute server URL")
	}
	requestOrigin, err := canonicalOrigin(parsed)
	if err != nil || requestOrigin != origin {
		return nil, newBridgeError(CodeForbiddenTarget, "request URL does not target the configured server")
	}
	if !isAllowedPath(parsed) {
		return nil, newBridgeError(CodeForbiddenTarget, "request path is outside the server API")
	}
	parsed.Fragment = ""
	return parsed, nil
}

// isAllowedPath reports whether a parsed request URL stays inside the API
// namespace or names the readiness probe.
func isAllowedPath(parsed *url.URL) bool {
	escapedPath := strings.ToLower(parsed.EscapedPath())
	for _, encoded := range []string{"%2e", "%2f", "%5c"} {
		if strings.Contains(escapedPath, encoded) {
			return false
		}
	}
	requestPath := parsed.Path
	if strings.Contains(requestPath, "\\") {
		return false
	}
	cleaned := path.Clean(requestPath)
	if cleaned != requestPath && cleaned+"/" != requestPath {
		return false
	}
	return requestPath == readinessPath || strings.HasPrefix(requestPath, apiPathPrefix)
}
