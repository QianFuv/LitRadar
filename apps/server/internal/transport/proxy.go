package transport

import (
	"errors"
	"log/slog"
	"net/http"
	"net/url"
	"strings"

	platform "github.com/QianFuv/LitRadar/apps/server/internal/platform/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ErrProxyURL contains no upstream authority or credentials.
var ErrProxyURL = errors.New("invalid Provider proxy URL")

// Proxy selects direct or explicit networking without ambient proxy discovery.
// Its zero value is direct and all implicit formatting redacts the configured URL.
type Proxy struct{ raw, canonical string }

// ExplicitProxy validates the original WHATWG URL boundary before constructing a Go transport.
func ExplicitProxy(value string) (Proxy, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return Proxy{}, ErrProxyURL
	}
	hasCompleteUserinfo, hasValidUserinfo := proxyUserinfo(location)
	if !hasValidUserinfo || !validProxyEndpoint(location) {
		return Proxy{}, ErrProxyURL
	}
	canonical := location.Href(false)
	if hasCompleteUserinfo {
		username, userError := url.PathUnescape(location.Username())
		password, passwordError := url.PathUnescape(location.Password())
		if userError != nil || passwordError != nil {
			return Proxy{}, ErrProxyURL
		}
		credentials := url.UserPassword(LossyUtf8([]byte(username)), LossyUtf8([]byte(password))).String()
		_, authority, _ := strings.Cut(canonical, "://")
		_, host, _ := strings.Cut(authority, "@")
		canonical = location.Scheme() + "://" + credentials + "@" + host
	}
	wire, err := platform.New(canonical)
	if err != nil {
		return Proxy{}, ErrProxyURL
	}
	wire.CloseIdleConnections()
	return Proxy{raw: value, canonical: canonical}, nil
}

// IsExplicit reports whether the decision has an explicit managed proxy.
func (proxy Proxy) IsExplicit() bool { return proxy.canonical != "" }

// URL returns the credential-bearing URL only for explicit bootstrap propagation.
func (proxy Proxy) URL() (string, bool) { return proxy.raw, proxy.IsExplicit() }

// Transport constructs a dedicated direct-or-explicit transport with no fallback.
func (proxy Proxy) Transport() (*http.Transport, error) {
	wire, err := platform.New(proxy.canonical)
	if err != nil {
		return nil, ErrProxyURL
	}
	return wire, nil
}

func (proxy Proxy) String() string {
	if proxy.IsExplicit() {
		return "ProviderProxy(mode=explicit, url=[REDACTED])"
	}
	return "ProviderProxy(mode=direct)"
}
func (proxy Proxy) GoString() string     { return proxy.String() }
func (proxy Proxy) LogValue() slog.Value { return slog.StringValue(proxy.String()) }

// proxyUserinfo retains the requirement that username and password are both present or both absent.
func proxyUserinfo(location *whatwg.Url) (bool, bool) {
	hasUserinfo := location.Username() != "" || location.Password() != ""
	hasCompleteUserinfo := location.Username() != "" && location.Password() != ""
	return hasCompleteUserinfo, hasUserinfo == hasCompleteUserinfo
}

// validProxyEndpoint retains the original host, port, path, query, fragment and opaque-path boundary.
func validProxyEndpoint(location *whatwg.Url) bool {
	return supportedProxyScheme(location.Scheme()) && location.Hostname() != "" && location.Port() != "0" && validProxyPath(location.Pathname()) && !strings.Contains(location.Href(true), "?") && location.Href(false) == location.Href(true) && !location.OpaquePath()
}

// supportedProxyScheme permits the four explicit provider transport schemes.
func supportedProxyScheme(scheme string) bool {
	return scheme == "http" || scheme == "https" || scheme == "socks5" || scheme == "socks5h"
}

// validProxyPath admits only an empty path or its canonical slash.
func validProxyPath(path string) bool {
	return path == "" || path == "/"
}
