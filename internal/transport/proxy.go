package transport

import (
	"errors"
	"log/slog"
	"net/http"
	"net/url"
	"strconv"
	"strings"

	platform "github.com/QianFuv/LitRadar/internal/platform/transport"
	whatwg "github.com/nlnwa/whatwg-url/url"
)

// ErrProxyUrl contains no upstream authority or credentials.
var ErrProxyUrl = errors.New("Invalid Provider proxy URL")

// Proxy selects direct or explicit networking without ambient proxy discovery.
// Its zero value is direct and all implicit formatting redacts the configured URL.
type Proxy struct{ raw, canonical string }

// ExplicitProxy validates the original WHATWG URL boundary before constructing a Go transport.
func ExplicitProxy(value string) (Proxy, error) {
	location, err := whatwg.NewParser().Parse(value)
	if err != nil {
		return Proxy{}, ErrProxyUrl
	}
	hasUserinfo := location.Username() != "" || location.Password() != ""
	hasCompleteUserinfo := location.Username() != "" && location.Password() != ""
	if location.Scheme() != "http" && location.Scheme() != "https" && location.Scheme() != "socks5" && location.Scheme() != "socks5h" || location.Hostname() == "" || hasUserinfo != hasCompleteUserinfo || location.Port() == "0" || location.Pathname() != "" && location.Pathname() != "/" || strings.Contains(location.Href(true), "?") || location.Href(false) != location.Href(true) || location.OpaquePath() {
		return Proxy{}, ErrProxyUrl
	}
	canonical := location.Href(false)
	if hasCompleteUserinfo {
		credentials := url.UserPassword(proxyCredential(location.Username()), proxyCredential(location.Password())).String()
		_, authority, _ := strings.Cut(canonical, "://")
		_, host, _ := strings.Cut(authority, "@")
		canonical = location.Scheme() + "://" + credentials + "@" + host
	}
	wire, err := platform.New(canonical)
	if err != nil {
		return Proxy{}, ErrProxyUrl
	}
	wire.CloseIdleConnections()
	return Proxy{raw: value, canonical: canonical}, nil
}

func proxyCredential(value string) string {
	decoded := make([]byte, 0, len(value))
	for index := 0; index < len(value); index++ {
		if value[index] == '%' && index+2 < len(value) {
			if character, err := strconv.ParseUint(value[index+1:index+3], 16, 8); err == nil {
				decoded = append(decoded, byte(character))
				index += 2
				continue
			}
		}
		decoded = append(decoded, value[index])
	}
	return LossyUtf8(decoded)
}

// IsExplicit reports whether the decision has an explicit managed proxy.
func (proxy Proxy) IsExplicit() bool { return proxy.canonical != "" }

// Url returns the credential-bearing URL only for explicit bootstrap propagation.
func (proxy Proxy) Url() (string, bool) { return proxy.raw, proxy.IsExplicit() }

// Transport constructs a dedicated direct-or-explicit transport with no fallback.
func (proxy Proxy) Transport() (*http.Transport, error) {
	wire, err := platform.New(proxy.canonical)
	if err != nil {
		return nil, ErrProxyUrl
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
