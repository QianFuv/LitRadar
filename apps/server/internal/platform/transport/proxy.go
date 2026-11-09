// Package transport supplies explicit network policy without ambient proxy inheritance.
package transport

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"golang.org/x/net/proxy"
)

// New creates an HTTP transport whose proxy is exclusively the supplied managed URL.
// SOCKS5 resolves destinations locally; SOCKS5h leaves destination resolution to the proxy.
func New(proxyUrl string) (*http.Transport, error) {
	dialer := &net.Dialer{Timeout: 30 * time.Second, KeepAlive: 30 * time.Second}
	transport := &http.Transport{DialContext: dialer.DialContext, ForceAttemptHTTP2: true, MaxIdleConns: 100,
		IdleConnTimeout: 90 * time.Second, TLSHandshakeTimeout: 10 * time.Second, ExpectContinueTimeout: time.Second}
	if proxyUrl == "" {
		return transport, nil
	}
	location, err := url.Parse(proxyUrl)
	invalid := errors.New("invalid proxy URL")
	if err != nil || !validProxyLocation(location, proxyUrl) {
		return nil, invalid
	}
	if !validProxyPort(location.Port()) {
		return nil, invalid
	}
	authentication, ok := proxyAuthentication(location)
	if !ok {
		return nil, invalid
	}
	switch location.Scheme {
	case "http", "https":
		transport.Proxy = http.ProxyURL(location)
	case "socks5", "socks5h":
		if err := configureSocksTransport(transport, location, authentication, dialer, invalid); err != nil {
			return nil, err
		}
	default:
		return nil, invalid
	}
	return transport, nil
}

// validProxyLocation retains the raw fragment and decoded path admission boundaries.
func validProxyLocation(location *url.URL, raw string) bool {
	return location.Hostname() != "" && location.Opaque == "" && location.RawQuery == "" && !location.ForceQuery && !strings.Contains(raw, "#") && (location.Path == "" || location.Path == "/")
}

// validProxyPort admits an omitted port or a nonzero unsigned sixteen-bit value.
func validProxyPort(port string) bool {
	if port == "" {
		return true
	}
	value, err := strconv.ParseUint(port, 10, 16)
	return err == nil && value != 0
}

// proxyAuthentication retains decoded credentials and the explicit nonempty password requirement.
func proxyAuthentication(location *url.URL) (*proxy.Auth, bool) {
	if location.User == nil {
		return nil, true
	}
	password, exists := location.User.Password()
	if location.User.Username() == "" || !exists || password == "" {
		return nil, false
	}
	return &proxy.Auth{User: location.User.Username(), Password: password}, true
}

// configureSocksTransport installs the selected DNS policy without changing transport deadlines.
func configureSocksTransport(transport *http.Transport, location *url.URL, authentication *proxy.Auth, dialer *net.Dialer, invalid error) error {
	port := location.Port()
	if port == "" {
		port = "1080"
	}
	selected, err := proxy.SOCKS5("tcp", net.JoinHostPort(location.Hostname(), port), authentication, dialer)
	if err != nil {
		return invalid
	}
	contextDialer := selected.(proxy.ContextDialer)
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return dialSocksTransport(ctx, network, address, location, contextDialer)
	}
	return nil
}

// dialSocksTransport preserves remote DNS and ordered local address attempts with cancellation precedence.
func dialSocksTransport(ctx context.Context, network, address string, location *url.URL, contextDialer proxy.ContextDialer) (net.Conn, error) {
	if location.Scheme == "socks5h" {
		return contextDialer.DialContext(ctx, network, address)
	}
	host, port, err := net.SplitHostPort(address)
	if err != nil {
		return nil, err
	}
	if net.ParseIP(host) != nil {
		return contextDialer.DialContext(ctx, network, address)
	}
	addresses, err := net.DefaultResolver.LookupIPAddr(ctx, host)
	if err != nil {
		return nil, err
	}
	var failures []error
	for _, resolved := range addresses {
		connection, err := contextDialer.DialContext(ctx, network, net.JoinHostPort(resolved.String(), port))
		if err == nil {
			return connection, nil
		}
		failures = append(failures, err)
		if ctx.Err() != nil {
			return nil, ctx.Err()
		}
	}
	if len(failures) == 0 {
		return nil, errors.New("destination has no addresses")
	}
	return nil, errors.Join(failures...)
}
