package transport

import (
	"context"
	"errors"
	"net"
	"net/http"
	"net/url"
	"time"

	"golang.org/x/net/proxy"
)

// ClientTransport supplies source headers and delegates gzip decoding to net/http.
// Request contexts own connection and handshake deadlines, including proxy negotiation.
type ClientTransport struct{ *http.Transport }

// ClientTransport constructs source networking without shorter implicit dial timeouts.
func (proxy Proxy) ClientTransport() (*ClientTransport, error) {
	wire, err := proxy.Transport()
	if err != nil {
		return nil, err
	}
	dialer := &net.Dialer{KeepAlive: 30 * time.Second}
	wire.DialContext = dialer.DialContext
	wire.TLSHandshakeTimeout = 0
	wire.DisableCompression = false
	if proxy.IsExplicit() {
		location, err := url.Parse(proxy.canonical)
		if err != nil {
			return nil, ErrProxyURL
		}
		if location.Scheme == "socks5" || location.Scheme == "socks5h" {
			if err := configureSocksDialer(wire, location, dialer); err != nil {
				return nil, err
			}
		}
	}
	return &ClientTransport{wire}, nil
}

// RoundTrip adds source defaults and leaves decoding failures to bounded body readers.
func (wire *ClientTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	request = request.Clone(request.Context())
	if _, exists := request.Header["User-Agent"]; !exists {
		request.Header.Set("User-Agent", "")
	}
	if _, exists := request.Header["Accept"]; !exists {
		request.Header.Set("Accept", "*/*")
	}

	return wire.Transport.RoundTrip(request)
}

// configureSocksDialer installs explicit proxy authentication and the selected DNS policy.
func configureSocksDialer(wire *http.Transport, location *url.URL, dialer *net.Dialer) error {
	var authentication *proxy.Auth
	if location.User != nil {
		password, _ := location.User.Password()
		authentication = &proxy.Auth{User: location.User.Username(), Password: password}
	}
	port := location.Port()
	if port == "" {
		port = "1080"
	}
	selected, err := proxy.SOCKS5("tcp", net.JoinHostPort(location.Hostname(), port), authentication, dialer)
	if err != nil {
		return ErrProxyURL
	}
	contextDialer := selected.(proxy.ContextDialer)
	wire.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		return dialSocksDestination(ctx, network, address, location, contextDialer)
	}

	return nil
}

// dialSocksDestination retains remote DNS or ordered local address attempts under the request context.
func dialSocksDestination(ctx context.Context, network, address string, location *url.URL, contextDialer proxy.ContextDialer) (net.Conn, error) {
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
	failures := []error{}
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
